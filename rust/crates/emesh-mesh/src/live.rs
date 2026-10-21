//! Live mesh: an instant preview while editing, replaced by Eilmer's own mesh once
//! `lmr custom-script` (run in the background after each committed edit) has written it.
//!
//! Everything runs on one worker thread so the UI never waits: the UI sends jobs (the document
//! with concrete blocks, tagged with its revision) and polls for results. A newer job replaces
//! an older one that has not started, and kills an lmr run that has.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use emesh_core::document::Document;
use emesh_lua::MESH_FILE;

use crate::preview::{block_grid, Grid};
use crate::runner::{clear_preview, error_summary, find_lmr, preview_files, write_scripts};
use crate::vtk::read_structured_vtk;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Preview,
    Eilmer,
}

/// Grids for one sketch revision: (block id, grid).
#[derive(Clone, Debug)]
pub struct Mesh {
    pub revision: u64,
    pub grids: Vec<(String, Grid)>,
    pub source: Source,
}

impl Mesh {
    pub fn cells(&self) -> usize {
        self.grids.iter().map(|(_, g)| g.cells()).sum()
    }
}

/// Preview grids of every block (blocks whose geometry is degenerate are skipped).
pub fn preview(doc: &Document, revision: u64) -> Mesh {
    let mut grids = vec![];
    for b in &doc.blocks {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_grid(doc, b)));
        if let Ok(g) = r {
            if g.pts.iter().all(|p| p[0].is_finite() && p[1].is_finite()) {
                grids.push((b.id.clone(), g));
            }
        }
    }
    Mesh { revision, grids, source: Source::Preview }
}

pub enum Event {
    Mesh(Mesh),
    /// one line for the status bar
    Status(String),
}

enum Job {
    Preview { revision: u64, doc: Document },
    Eilmer { revision: u64, doc: Document },
    Workdir(Option<PathBuf>),
}

pub struct LiveMesher {
    tx: Sender<Job>,
    rx: Receiver<Event>,
}

const EILMER_DELAY: Duration = Duration::from_millis(150);

impl LiveMesher {
    /// `wake` is called whenever an event is ready (e.g. to request a repaint).
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> LiveMesher {
        let (tx, jobs) = channel::<Job>();
        let (events, rx) = channel::<Event>();
        std::thread::Builder::new()
            .name("emesh-live".into())
            .spawn(move || Worker { jobs, events, wake, workdir: None, tmp: None }.run())
            .expect("cannot start the mesh worker thread");
        LiveMesher { tx, rx }
    }

    /// Recompute the preview for this revision (concrete blocks only).
    pub fn preview(&self, revision: u64, doc: Document) {
        let _ = self.tx.send(Job::Preview { revision, doc });
    }

    /// Run Eilmer on this revision (after a short delay; newer requests replace it).
    pub fn eilmer(&self, revision: u64, doc: Document) {
        let _ = self.tx.send(Job::Eilmer { revision, doc });
    }

    /// Where lmr runs (None: a temporary folder).
    pub fn set_workdir(&self, dir: Option<PathBuf>) {
        let _ = self.tx.send(Job::Workdir(dir));
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

struct Worker {
    jobs: Receiver<Job>,
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    workdir: Option<PathBuf>,
    tmp: Option<PathBuf>,
}

struct Pending {
    preview: Option<(u64, Document)>,
    eilmer: Option<(u64, Document, Instant)>,
}

impl Worker {
    fn send(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn dir(&mut self) -> PathBuf {
        if let Some(d) = &self.workdir {
            let _ = std::fs::create_dir_all(d);
            return d.clone();
        }
        if self.tmp.is_none() {
            let d = std::env::temp_dir().join(format!("emesh-live-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&d);
            self.tmp = Some(d);
        }
        self.tmp.clone().unwrap()
    }

    /// The app has gone: remove the temporary run folder.
    fn cleanup(&self) {
        if let Some(d) = &self.tmp {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    fn take(&mut self, job: Job, p: &mut Pending) {
        match job {
            Job::Preview { revision, doc } => p.preview = Some((revision, doc)),
            Job::Eilmer { revision, doc } => p.eilmer = Some((revision, doc, Instant::now() + EILMER_DELAY)),
            Job::Workdir(d) => self.workdir = d,
        }
    }

    fn run(mut self) {
        let mut p = Pending { preview: None, eilmer: None };
        loop {
            // wait for work
            let timeout = p.eilmer.as_ref().map(|e| e.2.saturating_duration_since(Instant::now()));
            let job = match timeout {
                None if p.preview.is_none() => match self.jobs.recv() {
                    Ok(j) => Some(j),
                    Err(_) => return self.cleanup(),
                },
                None => None,
                Some(t) => match self.jobs.recv_timeout(if p.preview.is_some() { Duration::ZERO } else { t }) {
                    Ok(j) => Some(j),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return self.cleanup(),
                },
            };
            if let Some(j) = job {
                self.take(j, &mut p);
            }
            while let Ok(j) = self.jobs.try_recv() {
                self.take(j, &mut p);
            }
            if let Some((rev, doc)) = p.preview.take() {
                let mesh = preview(&doc, rev);
                self.send(Event::Mesh(mesh));
                continue;
            }
            let due = p.eilmer.as_ref().is_some_and(|e| Instant::now() >= e.2);
            if due {
                let (rev, doc, _) = p.eilmer.take().unwrap();
                self.run_eilmer(rev, doc, &mut p);
            }
        }
    }

    fn run_eilmer(&mut self, rev: u64, doc: Document, p: &mut Pending) {
        if doc.blocks.is_empty() {
            return;
        }
        let dir = self.dir();
        let lmr = match find_lmr() {
            Ok(l) => l,
            Err(e) => {
                self.send(Event::Status(format!("Mesh: {e}")));
                return;
            }
        };
        if let Err(e) = write_scripts(&doc, &dir) {
            self.send(Event::Status(format!("Mesh: {e}")));
            return;
        }
        clear_preview(&dir);
        let ids: Vec<String> = doc.blocks.iter().map(|b| b.id.clone()).collect();
        self.send(Event::Status("Mesh: running Eilmer...".into()));
        let log = dir.join("lmr-output.txt");
        let out = std::fs::File::create(&log).ok();
        let mut cmd = Command::new(&lmr);
        cmd.args(["custom-script", "-j", MESH_FILE]).current_dir(&dir).stdin(Stdio::null());
        if let Some(f) = out {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f).stderr(f2);
            }
        }
        let mut child: Child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.send(Event::Status(format!("Mesh: cannot run lmr: {e}")));
                return;
            }
        };
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let code = status.code().unwrap_or(-1);
                    if code == 0 {
                        let grids: Result<Vec<Grid>, String> =
                            preview_files(&dir).iter().map(|f| read_structured_vtk(f)).collect();
                        match grids {
                            Ok(g) if g.len() == ids.len() => {
                                let mesh = Mesh { revision: rev, grids: ids.into_iter().zip(g).collect(), source: Source::Eilmer };
                                let cells = mesh.cells();
                                let n = mesh.grids.len();
                                self.send(Event::Mesh(mesh));
                                self.send(Event::Status(format!("Mesh: Eilmer, {n} block(s), {cells} cells")));
                            }
                            Ok(g) => self.send(Event::Status(format!(
                                "Mesh: Eilmer wrote {} grid(s) for {} block(s)",
                                g.len(),
                                ids.len()
                            ))),
                            Err(e) => self.send(Event::Status(format!("Mesh: {e}"))),
                        }
                    } else {
                        let output = std::fs::read_to_string(&log).unwrap_or_default();
                        self.send(Event::Status(format!(
                            "Mesh: Eilmer error: {}",
                            error_summary("custom-script", code, &output)
                        )));
                    }
                    return;
                }
                Ok(None) => {}
                Err(_) => return,
            }
            match self.jobs.recv_timeout(Duration::from_millis(10)) {
                Ok(j) => {
                    let newer_eilmer = matches!(j, Job::Eilmer { .. });
                    self.take(j, p);
                    if newer_eilmer {
                        let _ = child.kill(); // stale: a newer edit has been committed
                        let _ = child.wait();
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    return;
                }
            }
        }
    }
}
