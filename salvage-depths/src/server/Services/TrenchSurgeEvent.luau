--!strict
-- Trench Surge: shared pressure meter; players deliver cores (1 Exposed unit each) to relieve it.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local GameConfig = require(ReplicatedStorage.Shared.GameConfig)
local Remotes = require(ReplicatedStorage.Shared.Remotes)
local DataService = require(script.Parent.DataService)
local EconomyService = require(script.Parent.EconomyService)
local CargoService = require(script.Parent.CargoService)

local START_PRESSURE = 50
local RELIEF_PER_CORE = 3
local DELIVER_INTERVAL = 0.5
local SCORE_INTERVAL = 2
local TICK = 0.5

local TrenchSurgeEvent = {}

-- Returns scores per player and whether the Seal held (meter never hit 100).
function TrenchSurgeEvent.run(): ({ [Player]: number }, boolean)
	local ev = GameConfig.Event
	local rng = Random.new()
	local scores: { [Player]: number } = {}
	local lastDeliver: { [Player]: number } = {}
	local pressure = START_PRESSURE

	local conn = Remotes.event("DeliverCore").OnServerEvent:Connect(function(player: Player)
		local now = os.clock()
		if now - (lastDeliver[player] or 0) < DELIVER_INTERVAL then return end
		lastDeliver[player] = now
		if not DataService.isLoaded(player) then return end
		if CargoService.takeExposed(player, 1) < 1 then
			Remotes.event("Notify"):FireClient(player, { text = "You need cargo to power the Seal." })
			return
		end
		scores[player] = (scores[player] or 0) + 1
		pressure = math.max(0, pressure - RELIEF_PER_CORE)
	end)
	local leaveConn = Players.PlayerRemoving:Connect(function(p)
		lastDeliver[p] = nil
	end)

	local function broadcast()
		local ranked: { { name: string, score: number, player: Player } } = {}
		for p, s in scores do
			if p.Parent then table.insert(ranked, { name = p.DisplayName, score = s, player = p }) end
		end
		table.sort(ranked, function(a, b) return a.score > b.score end)
		local top = {}
		for i = 1, math.min(3, #ranked) do top[i] = { name = ranked[i].name, score = ranked[i].score } end
		for _, p in Players:GetPlayers() do
			Remotes.event("EventScore"):FireClient(p, { meter = math.floor(pressure), yourScore = scores[p] or 0, top = top })
		end
	end

	local elapsed, sinceScore = 0, SCORE_INTERVAL
	local sealed = true
	while elapsed < ev.DurationSeconds do
		task.wait(TICK)
		elapsed += TICK
		pressure = math.min(100, pressure + TICK * (0.3 + 0.03 * #Players:GetPlayers()))
		sinceScore += TICK
		if sinceScore >= SCORE_INTERVAL then
			sinceScore = 0
			broadcast()
		end
		if pressure >= 100 then sealed = false; break end
		if pressure <= 0 then break end
	end
	conn:Disconnect()
	leaveConn:Disconnect()
	broadcast()

	-- Rewards (participants still in server only).
	local ranked: { { p: Player, s: number } } = {}
	for p, s in scores do
		if p.Parent == Players and s >= ev.ParticipationThreshold then table.insert(ranked, { p = p, s = s }) end
	end
	table.sort(ranked, function(a, b) return a.s > b.s end)
	for rank, r in ranked do
		if sealed then
			EconomyService.addSoft(r.p, math.min(1000, 100 + 25 * r.s))
			EconomyService.addGems(r.p, rng:NextInteger(5, 15))
			if rank <= 3 then EconomyService.addSpecial(r.p, if rank == 1 then 2 else 1) end
		else
			EconomyService.addSoft(r.p, 25 + 5 * r.s)
		end
		Remotes.event("Notify"):FireClient(r.p, {
			text = if sealed then "The Seal held! Rewards granted." else "The Seal failed, but thanks for helping.",
		})
	end
	Remotes.event("EventAnnounce"):FireAllClients({ ended = true, sealed = sealed })
	return scores, sealed
end

return TrenchSurgeEvent
