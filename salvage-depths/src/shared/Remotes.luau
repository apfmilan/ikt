--!strict
-- Single source of truth for remote names. Server creates them; client waits for them.
local RunService = game:GetService("RunService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local EVENTS = {
	-- server -> client
	"BalanceUpdate",   -- { soft: number, gems: number, special: number }
	"CargoUpdate",     -- { exposed: number, secured: number, capacity: number }
	"DiveUpdate",      -- { state: string, depth: number, oxygen: number }
	"RaidWarning",     -- { attackerUserId: number, secondsLeft: number } | { cleared: true }
	"EventAnnounce",   -- { name: string, startsIn: number, duration: number } | { ended: true }
	"EventScore",      -- { meter: number (0..100), yourScore: number, top: { { name: string, score: number } } }
	"TutorialStep",    -- { step: number, id: string, text: string }
	"Notify",          -- { text: string }
	"PeacefulState",   -- boolean
	-- client -> server
	"DiveRequest",     -- "Dive" | "Surface"
	"SalvageRequest",  -- wreckId: string
	"DockRequest",     -- no args
	"TetherFire",      -- targetUserId: number
	"TetherCut",       -- no args
	"PlaceDefense",    -- kind: string
	"DeliverCore",     -- no args (Trench Surge)
	"TutorialAck",     -- stepId: string
	"PeacefulToggle",  -- boolean
	"ClientReady",     -- no args
}

local FUNCTIONS = {
	"GetProfileSnapshot", -- returns { soft, gems, special, tutorialStep, defenses, peaceful, securedCargo }
}

local Remotes = {}
local folder: Folder

if RunService:IsServer() then
	folder = Instance.new("Folder")
	folder.Name = "Remotes"
	for _, n in EVENTS do
		local r = Instance.new("RemoteEvent"); r.Name = n; r.Parent = folder
	end
	for _, n in FUNCTIONS do
		local r = Instance.new("RemoteFunction"); r.Name = n; r.Parent = folder
	end
	folder.Parent = ReplicatedStorage
else
	folder = ReplicatedStorage:WaitForChild("Remotes") :: Folder
end

function Remotes.event(name: string): RemoteEvent
	return folder:WaitForChild(name) :: RemoteEvent
end

function Remotes.func(name: string): RemoteFunction
	return folder:WaitForChild(name) :: RemoteFunction
end

return Remotes
