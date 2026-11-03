--!strict
-- Defenses are bought with SOFT only. Also exposes tether-defense hooks used by TetherRaidService.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local GameConfig = require(ReplicatedStorage.Shared.GameConfig)
local Remotes = require(ReplicatedStorage.Shared.Remotes)
local DataService = require(script.Parent.DataService)
local EconomyService = require(script.Parent.EconomyService)
local TutorialService = require(script.Parent.TutorialService)

type Def = { MaxTier: number, CostPerTier: { number } }
local defs = GameConfig.DefenseDefs :: any
local DEFS: { [string]: Def } = defs

local DECOY_CHANCE_PER_TIER = 0.15
local MINE_COOLDOWN = { 45, 35, 25 }
local DRONE_COOLDOWN = 20
local DRONE_DELAY = { 5, 3 }

local DefenseService = {}
local lastReq: { [Player]: number } = {}
local mineReadyAt: { [Player]: number } = {}
local droneReadyAt: { [Player]: number } = {}
local listeners: { (Player, string, number) -> () } = {}
local conns: { RBXScriptConnection } = {}

function DefenseService.getTier(player: Player, kind: string): number
	local data = DataService.get(player)
	if not data then return 0 end
	return data.Defenses[kind] or 0
end

function DefenseService.onChanged(cb: (Player, string, number) -> ())
	table.insert(listeners, cb)
end

function DefenseService.getDecoyChance(player: Player): number
	return DefenseService.getTier(player, "DecoyBuoy") * DECOY_CHANCE_PER_TIER
end

-- Returns true if a sonar mine fires (jams the attacker's lock); arms its cooldown.
function DefenseService.tryMine(player: Player): boolean
	local tier = DefenseService.getTier(player, "SonarMine")
	if tier <= 0 or os.clock() < (mineReadyAt[player] or 0) then return false end
	mineReadyAt[player] = os.clock() + (MINE_COOLDOWN[tier] or 45)
	return true
end

-- Returns seconds until the drone auto-cuts a new tether, or nil if no drone/on cooldown.
function DefenseService.tryDrone(player: Player): number?
	local tier = DefenseService.getTier(player, "GuardDrone")
	if tier <= 0 or os.clock() < (droneReadyAt[player] or 0) then return nil end
	droneReadyAt[player] = os.clock() + DRONE_COOLDOWN
	return DRONE_DELAY[tier] or 5
end

local function onPlace(player: Player, kind: any)
	local now = os.clock()
	if now - (lastReq[player] or 0) < 0.5 then return end
	lastReq[player] = now
	if type(kind) ~= "string" or #kind > 32 then return end
	local def = DEFS[kind]
	if not def or not DataService.isLoaded(player) then return end
	local data = DataService.get(player)
	if not data then return end
	local tier = data.Defenses[kind] or 0
	if tier >= def.MaxTier then
		Remotes.event("Notify"):FireClient(player, { text = kind .. " is at max tier." })
		return
	end
	local cost = def.CostPerTier[tier + 1]
	if not cost or not EconomyService.spendSoft(player, cost) then
		Remotes.event("Notify"):FireClient(player, { text = "Not enough Scrap." })
		return
	end
	data.Defenses[kind] = tier + 1
	Remotes.event("Notify"):FireClient(player, { text = kind .. " upgraded to tier " .. (tier + 1) .. "." })
	for _, cb in listeners do cb(player, kind, tier + 1) end
	TutorialService.advance(player, "PlaceDefense")
end

function DefenseService.init()
	table.insert(conns, Remotes.event("PlaceDefense").OnServerEvent:Connect(onPlace))
	table.insert(conns, Players.PlayerRemoving:Connect(function(p)
		lastReq[p] = nil; mineReadyAt[p] = nil; droneReadyAt[p] = nil
	end))
end

return DefenseService
