--!strict
-- Per-player dive state machine; oxygen/depth computed server-side from character position.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local Workspace = game:GetService("Workspace")

local GameConfig = require(ReplicatedStorage.Shared.GameConfig)
local Remotes = require(ReplicatedStorage.Shared.Remotes)
local StateMachine = require(ReplicatedStorage.Shared.StateMachine)
local DataService = require(script.Parent.DataService)
local DefenseService = require(script.Parent.DefenseService)
local TutorialService = require(script.Parent.TutorialService)

local TICK = 0.5
local BASE_DRAIN = 0.2
local DOCK_DEPTH = 3       -- surfacing below this depth auto-docks
local CAN_DOCK_DEPTH = 5

type Session = { machine: StateMachine.Machine?, oxygen: number, depth: number, tutorialSent: boolean }

local DiveService = {}
local sessions: { [Player]: Session } = {}
local lastReq: { [Player]: number } = {}
local conns: { RBXScriptConnection } = {}

local ALLOWED: { [string]: { [string]: boolean } } = {
	Docked = { Diving = true },
	Diving = { Salvaging = true, Surfacing = true },
	Salvaging = { Diving = true, Surfacing = true },
	Surfacing = { Docked = true, Diving = true },
}

local function surfaceY(): number
	local v = Workspace:GetAttribute("SurfaceY")
	return if type(v) == "number" then v else 0
end

local function getRoot(p: Player): BasePart?
	local c = p.Character
	if not c then return nil end
	local r = c:FindFirstChild("HumanoidRootPart")
	if r and r:IsA("BasePart") then return r end
	return nil
end

local function push(player: Player, s: Session)
	if not s.machine then return end
	Remotes.event("DiveUpdate"):FireClient(player, {
		state = s.machine.current,
		depth = math.floor(s.depth * 10) / 10,
		oxygen = math.floor(s.oxygen * 10) / 10,
	})
end

local function newSession(player: Player)
	local s: Session = { machine = nil, oxygen = GameConfig.Dive.MaxOxygen, depth = 0, tutorialSent = false }
	local function mk(name: string): StateMachine.State
		return { onEnter = function()
			if name == "Docked" then s.depth = 0; s.oxygen = GameConfig.Dive.MaxOxygen end
			push(player, s)
		end }
	end
	s.machine = StateMachine.new({
		Docked = mk("Docked"), Diving = mk("Diving"), Salvaging = mk("Salvaging"), Surfacing = mk("Surfacing"),
	}, "Docked")
	sessions[player] = s
	push(player, s)
end

local function transition(player: Player, to: string): boolean
	local s = sessions[player]
	if not s or not s.machine then return false end
	if not ALLOWED[s.machine.current][to] then return false end
	s.machine:go(to)
	return true
end

function DiveService.getState(player: Player): string
	local s = sessions[player]
	return if s and s.machine then s.machine.current else "Docked"
end
function DiveService.getDepth(player: Player): number
	local s = sessions[player]; return if s then s.depth else 0
end
function DiveService.getOxygen(player: Player): number
	local s = sessions[player]; return if s then s.oxygen else 0
end
function DiveService.getMaxDepth(player: Player): number
	return GameConfig.Dive.BaseMaxDepth
		+ GameConfig.Dive.DepthPerPlateTier * DefenseService.getTier(player, "HullPlating")
end

function DiveService.canDock(player: Player): boolean
	local s = sessions[player]
	if not s or not s.machine then return false end
	return s.machine.current == "Docked" or s.depth <= CAN_DOCK_DEPTH
end

function DiveService.dock(player: Player)
	local s = sessions[player]
	if not s or not s.machine or s.machine.current == "Docked" then return end
	if s.machine.current ~= "Surfacing" then s.machine:go("Surfacing") end
	s.machine:go("Docked")
end

-- Used by SalvageService; Salvaging <-> Diving only.
function DiveService.setSalvaging(player: Player, on: boolean): boolean
	return transition(player, if on then "Salvaging" else "Diving")
end

local function onRequest(player: Player, arg: any)
	local now = os.clock()
	if now - (lastReq[player] or 0) < 0.5 then return end
	lastReq[player] = now
	if (arg ~= "Dive" and arg ~= "Surface") or not DataService.isLoaded(player) then return end
	if arg == "Dive" then
		transition(player, "Diving")
	else
		transition(player, "Surfacing")
	end
end

local function step(player: Player, s: Session)
	local m = s.machine
	if not m or m.current == "Docked" then return end
	local root = getRoot(player)
	if root then
		local raw = surfaceY() - root.Position.Y
		local cap = DiveService.getMaxDepth(player)
		if raw > cap then
			root.CFrame += Vector3.new(0, raw - cap, 0)
			root.AssemblyLinearVelocity = Vector3.zero
			raw = cap
			Remotes.event("Notify"):FireClient(player, { text = "Hull limit reached. Upgrade Hull Plating to dive deeper." })
		end
		s.depth = math.max(0, raw)
	end
	s.oxygen = math.max(0, s.oxygen - TICK * (BASE_DRAIN + GameConfig.Dive.OxygenPerSecondPerMeter * s.depth))
	if s.oxygen <= 0 and (m.current == "Diving" or m.current == "Salvaging") then
		Remotes.event("Notify"):FireClient(player, { text = "Out of oxygen! Surface now." })
		m:go("Surfacing")
	end
	if m.current == "Surfacing" and s.depth <= DOCK_DEPTH then
		m:go("Docked")
		return
	end
	if not s.tutorialSent and s.depth >= 20 then
		s.tutorialSent = true
		TutorialService.advance(player, "Dive")
	end
	push(player, s)
end

function DiveService.init()
	for _, p in Players:GetPlayers() do newSession(p) end
	table.insert(conns, Players.PlayerAdded:Connect(newSession))
	table.insert(conns, Players.PlayerRemoving:Connect(function(p)
		sessions[p] = nil; lastReq[p] = nil
	end))
	table.insert(conns, Remotes.event("DiveRequest").OnServerEvent:Connect(onRequest))
	task.spawn(function()
		while true do
			task.wait(TICK)
			for p, s in sessions do step(p, s) end
		end
	end)
end

return DiveService
