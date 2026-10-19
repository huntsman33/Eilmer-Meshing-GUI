"""
Boundary conditions: the Eilmer BC classes offered in the GUI, tag rules, and
the starter simulation script written on export.

A solid curve's bc_tag names an entry in Document.bcs: {"class": ..., "args": ...}.
Untagged faces get Eilmer's default, WallBC_WithSlip. The tags go into each
grid's bcTags in geometry.lua; bcs.lua maps them to BC objects (bcDict) for
`lmr prep-sim`.
"""
import re

# (class, group, default constructor arguments, default tag, description)
BC_CLASSES = [
    ("WallBC_WithSlip", "Wall", "", "slipwall", "Inviscid wall (slip); the default for untagged faces"),
    ("WallBC_NoSlip_FixedT", "Wall", "Twall=300.0", "wall", "Viscous wall at a fixed temperature"),
    ("WallBC_NoSlip_Adiabatic", "Wall", "", "wall", "Viscous adiabatic wall"),
    ("InFlowBC_Supersonic", "Inflow", "flowState=inflow", "inflow", "Supersonic inflow: every property fixed"),
    ("InFlowBC_FromStagnation", "Inflow", "stagnationState=stagnation", "inflow",
     "Subsonic inflow from a stagnation (reservoir) state"),
    ("InFlowBC_StaticProfile", "Inflow", 'fileName="inflow-profile.dat"', "inflow",
     "Inflow from a profile file"),
    ("InOutFlowBC_Ambient", "In/out", "flowState=ambient", "ambient",
     "Ambient conditions; the flow may go in or out"),
    ("OutFlowBC_Simple", "Outflow", "", "outflow", "Supersonic outflow (extrapolates the flux)"),
    ("OutFlowBC_SimpleExtrapolate", "Outflow", "", "outflow", "Outflow extrapolating the flow state"),
    ("OutFlowBC_FixedP", "Outflow", "p_outside=101.325e3", "outflow", "Subsonic outflow at a fixed pressure"),
    ("OutFlowBC_FixedPT", "Outflow", "p_outside=101.325e3, T_outside=300.0", "outflow",
     "Subsonic outflow at a fixed pressure and temperature"),
    ("UserDefinedBC", "Other", 'fileName="udf-bc.lua"', "udf", "User-defined ghost-cell BC (Lua file)"),
]
BC_BY_CLASS = {c[0]: c for c in BC_CLASSES}

TAG_COLORS = ["#d43f3a", "#2e9d57", "#8e5bd0", "#c9a227", "#1a73d9", "#e0679b", "#17a2b8", "#7f5a2e"]

_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_LUA_KEYWORDS = {"and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if",
                 "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"}


def check_tag(tag):
    """Tags are bcDict keys and bcTags strings, so they must be Lua identifiers."""
    if not _IDENT.match(tag or "") or tag in _LUA_KEYWORDS:
        raise ValueError(f"'{tag}' is not a valid tag: use letters, digits and _, starting with a letter")
    return tag


def tag_color(doc, tag):
    tags = sorted(doc.bcs)
    return TAG_COLORS[tags.index(tag) % len(TAG_COLORS)] if tag in tags else TAG_COLORS[0]


def flow_state_names(args):
    """Lua names a BC's arguments refer to (e.g. flowState=inflow -> inflow)."""
    names = []
    for _, value in re.findall(r"(\w+)\s*=\s*([A-Za-z_]\w*)", args or ""):
        if value not in _LUA_KEYWORDS and value not in names:
            names.append(value)
    return names


SIM_TEMPLATE = """\
-- sim.lua: a starting point for simulating the grids exported by eilmer-mesher.
-- Edit the gas model, flow states and run settings, then:
--   lmr prep-gas -i ideal-air.lua -o ideal-air.gas   (or use your own gas file)
--   lmr prep-grids -j geometry.lua
--   lmr prep-sim -j sim.lua
--   lmr run
config.solver_mode = "transient"
config.dimensions = 2
config.axisymmetric = false
setGasModel("ideal-air.gas")

-- Flow states. 'initial' fills the domain (every block has fsTag "initial");
-- the others are referred to by the boundary conditions in bcs.lua.
initial = FlowState:new{{p=100.0e3, T=300.0}}
{states}
flowDict = {{initial=initial}}

dofile("bcs.lua")                 -- bcDict: the boundary-condition tags set in the GUI
makeFluidBlocks(bcDict, flowDict)

config.max_time = 1.0e-3          -- seconds
config.max_step = 100000
config.dt_init = 1.0e-9
config.cfl_value = 0.5
config.dt_plot = config.max_time / 20
"""

GAS_TEMPLATE = """\
-- ideal-air.lua: gas model input for `lmr prep-gas -i ideal-air.lua -o ideal-air.gas`
model = "IdealGas"
species = {'air'}
"""


def sim_template(doc, tags):
    """Starter sim.lua defining a placeholder FlowState for every name the BCs refer to."""
    states = []
    for tag in tags:
        spec = doc.bcs.get(tag, {})
        for name in flow_state_names(spec.get("args", "")):
            if name != "initial" and name not in [s.split()[0] for s in states]:
                states.append(f"{name} = FlowState:new{{p=100.0e3, T=300.0, velx=0.0}}"
                              f"   -- used by {spec.get('class', '?')} ({tag})")
    return SIM_TEMPLATE.format(states="\n".join(states) or "-- (no boundary condition needs a flow state)")
