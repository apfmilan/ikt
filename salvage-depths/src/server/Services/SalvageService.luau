--!strict
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Remotes = require(ReplicatedStorage.Shared.Remotes)
local DataService = require(script.Parent.DataService)
local EconomyService = require(script.Parent.EconomyService)
local TutorialService = require(script.Parent.TutorialService)
local SectorService = require(script.Parent.SectorService)
local DiveService = require(script.Parent.DiveService)
local CargoService = require(script.Parent.CargoService)

local SALVAGE_RANGE = 30
local SALVAGE_TIME = 0.8
local MIN_INTERVAL = SALVAGE_TIME -- server-enforced: never faster than one salvage animation
local MAX_SPEED = 120

local SalvageService = {}
local lastReq: { [Player]: number } = {}
local lastSalvage: { [Player]: { pos: Vector3, t: number } } = {}
local conns: { RBXScriptConnection } = {}

local function onSalvage(player: Player, wreckId: any)
	local now = os.clock()
	if now - (lastReq[player] or 0) < MIN_INTERVAL then return end
	lastReq[player] = now
	if type(wreckId) ~= "string" or #wreckId > 16 or not DataService.isLoaded(player) then return end
	if DiveService.getState(player) ~= "Diving" then return end

	local wreck = SectorService.getWreck(wreckId)
	if not wreck then return end
	local char = player.Character
	local root = char and char:FindFirstChild("HumanoidRootPart")
	if not root or not root:IsA("BasePart") then return end
	if (root.Position - wreck.part.Position).Magnitude > SALVAGE_RANGE then return end
	if wreck.depth > DiveService.getMaxDepth(player) + 10 then return end
	local prev = lastSalvage[player]
	if prev and (root.Position - prev.pos).Magnitude / math.max(now - prev.t, 1e-3) > MAX_SPEED then return end

	local relic = wreck.relic
	local value = SectorService.claimWreck(wreckId)
	if not value then return end

	lastSalvage[player] = { pos = root.Position, t = now }
	DiveService.setSalvaging(player, true)
	task.delay(SALVAGE_TIME, function()
		if player.Parent and DiveService.getState(player) == "Salvaging" then
			DiveService.setSalvaging(player, false)
		end
	end)

	CargoService.addExposed(player, value)
	if relic then EconomyService.addSpecial(player, 1) end
	Remotes.event("Notify"):FireClient(player, {
		text = if relic then ("Relic salvaged! +%d scrap, +1 Artifact"):format(value) else ("Salvaged +%d scrap"):format(value),
	})
	TutorialService.advance(player, "Salvage")
end

function SalvageService.init()
	table.insert(conns, Remotes.event("SalvageRequest").OnServerEvent:Connect(onSalvage))
	table.insert(conns, Players.PlayerRemoving:Connect(function(p) lastReq[p] = nil; lastSalvage[p] = nil end))
end

return SalvageService
