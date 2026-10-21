-- sim.lua: a starting point for simulating the grids exported by eilmer-mesher.
-- Edit the gas model, flow states and run settings, then:
--   lmr prep-gas -i ideal-air.lua -o ideal-air.gas   (or use your own gas file)
--   lmr prep-grids -j mesh.lua
--   lmr prep-sim -j sim.lua
--   lmr run
--   lmr snapshot2vtk -a
--   cd lmrsim/vtk && paraview fluid.pvd
setGasModel("ideal-air.gas")
config.solver_mode = "transient"
config.gasdynamic_update_scheme  = "predictor-corrector"
config.flux_calculator = "adaptive_hanel_ausmdv"
config.dimensions = 2
config.axisymmetric = false

-- Flow states. 'initial' fills the domain (every block has fsTag "initial");
-- the others are referred to by the boundary conditions in bcs.lua.
p_inf = 1.5e3 -- Pa
T_inf = 227.0 -- K
M_inf = 6.0
tmp = FlowState:new{p=p_inf, T=T_inf}
u_inf = M_inf * tmp.a -- m/s

initial = FlowState:new{p=p_inf/5.0, T=T_inf}          -- used by all blocks (fsTag="initial")
inflow = FlowState:new{p=p_inf, T=T_inf, velx=u_inf}   -- free stream, for an inflow boundary condition

flowDict = {initial=initial, inflow=inflow}

dofile("bcs.lua")               -- bcDict: the boundary-condition tags set in the GUI
makeFluidBlocks(bcDict, flowDict)

config.max_time = 1e-3          -- seconds
config.max_step = 100000
config.dt_init = 1.0e-9
config.cfl_value = 0.5
config.dt_plot = config.max_time / 100
