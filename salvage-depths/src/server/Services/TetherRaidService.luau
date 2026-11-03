--!strict
-- Server-authoritative Cargo Tether raids. Takes only from the victim's Exposed hold.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local GameConfig = require(ReplicatedStorage.Shared.GameConfig)
local Remotes = require(ReplicatedStorage.Shared.Remotes)
local DataService = require(script.Parent.DataService)
local TutorialService = require(script.Parent.TutorialService)
local DiveService = require(script.Parent.DiveService)
local CargoService = require(script.Parent.CargoService)
local DefenseService = require(script.Parent.DefenseService)

local Raid = GameConfig.Raid
local DRILL_SECONDS = 4
local MIN_CUT_HOLD = 1       -- seconds a tether must exist before a cut counts
local MINE_JAM_SECONDS = 5
local DECOY_LOCKOUT = 3
local TOGGLE_COOLDOWN = 30
local MIN_EXPOSED = 10

type Tether = { attacker: Player, target: Player?, startedAt: number, cancelled: boolean }

local TetherRaidService = {}
local byAttacker: { [Player]: Tether } = {}
local byTarget: { [Player]: Tether } = {}
local lockedUntil: { [Player]: number } = {}
local lastFire: { [Player]: number } = {}
local lastCut: { [Player]: number } = {}
local lastToggle: { [Player]: number } = {}
local conns: { RBXScriptConnection } = {}

local function notify(p: Player, text: string)
	Remotes.event("Notify"):FireClient(p, { text = text })
end

local function getRoot(p: Player): BasePart?
	local c = p.Character
	local r = c and c:FindFirstChild("HumanoidRootPart")
	if r and r:IsA("BasePart") then return r end
	return nil
end

local function inRange(a: Player, b: Player): boolean
	local ra, rb = getRoot(a), getRoot(b)
	return ra ~= nil and rb ~= nil and (ra.Position - rb.Position).Magnitude <= Raid.TetherRange
end

local function finish(t: Tether)
	if t.cancelled then return end
	t.cancelled = true
	byAttacker[t.attacker] = nil
	local target = t.target
	if target then
		byTarget[target] = nil
		if target.Parent then Remotes.event("RaidWarning"):FireClient(target, { cleared = true }) end
	else
		if t.attacker.Parent then Remotes.event("RaidWarning"):FireClient(t.attacker, { cleared = true }) end
	end
end

local function stillValid(t: Tether, target: Player): boolean
	local a = t.attacker
	if a.Parent ~= Players or target.Parent ~= Players then return false end
	if not DataService.isLoaded(a) or not DataService.isLoaded(target) then return false end
	if DiveService.getState(target) == "Docked" or DiveService.getState(a) == "Docked" then return false end
	return inRange(a, target)
end

local function resolve(t: Tether, target: Player)
	local attacker = t.attacker
	local vdata = DataService.get(target)
	if not vdata then return end
	local now = os.time()
	local loss = math.floor(CargoService.getExposed(target) * Raid.MaxLossPct)
	local taken = CargoService.takeExposed(target, loss) -- Exposed only; Secured/Special untouched
	if taken > 0 then
		CargoService.addExposed(attacker, taken)
		local refund = math.floor(taken * Raid.VictimRefundPct)
		if refund > 0 then CargoService.addExposed(target, refund) end
	end
	vdata.LastRaidedAt = now
	vdata.RaidedBy[tostring(attacker.UserId)] = now
	notify(attacker, ("Tether complete. Stole %d scrap."):format(taken))
	notify(target, ("%s raided you for %d scrap (%d refunded). Protected for a while."):format(
		attacker.DisplayName, taken, math.floor(taken * Raid.VictimRefundPct)))
end

local function holdLoop(t: Tether, target: Player)
	local droneDelay = DefenseService.tryDrone(target)
	local lastSent = -1
	while true do
		task.wait(0.25)
		if t.cancelled then return end
		if not stillValid(t, target) then
			notify(t.attacker, "Tether lost.")
			finish(t)
			return
		end
		local elapsed = os.clock() - t.startedAt
		if droneDelay and elapsed >= droneDelay then
			notify(t.attacker, "Guard Drone cut your tether!")
			notify(target, "Guard Drone cut the tether.")
			finish(t)
			return
		end
		local left = Raid.TetherHoldSeconds - elapsed
		if left <= 0 then
			finish(t)
			resolve(t, target)
			return
		end
		local whole = math.ceil(left)
		if whole ~= lastSent then
			lastSent = whole
			Remotes.event("RaidWarning"):FireClient(target, { attackerUserId = t.attacker.UserId, secondsLeft = whole })
		end
	end
end

local function startDrill(attacker: Player)
	local t: Tether = { attacker = attacker, target = nil, startedAt = os.clock(), cancelled = false }
	byAttacker[attacker] = t
	Remotes.event("RaidWarning"):FireClient(attacker, { attackerUserId = 0, secondsLeft = DRILL_SECONDS })
	task.delay(DRILL_SECONDS, function()
		if t.cancelled then return end
		finish(t)
		if attacker.Parent then
			notify(attacker, "Drill complete. Real raids take 8s and you can cut the tether.")
			TutorialService.advance(attacker, "TetherDrill")
		end
	end)
end

local function onFire(attacker: Player, targetUserId: any)
	local now = os.clock()
	if now - (lastFire[attacker] or 0) < 1 then return end
	lastFire[attacker] = now
	if type(targetUserId) ~= "number" or targetUserId ~= targetUserId or targetUserId < 0
		or targetUserId ~= math.floor(targetUserId) or targetUserId > 2 ^ 52 then return end
	if byAttacker[attacker] or byTarget[attacker] or not DataService.isLoaded(attacker) then return end

	if targetUserId == 0 then
		startDrill(attacker)
		return
	end

	local adata = DataService.get(attacker)
	if not adata or adata.Peaceful then
		notify(attacker, "Peaceful mode is on. Turn it off to raid.")
		return
	end
	if now < (lockedUntil[attacker] or 0) then return end
	local target = Players:GetPlayerByUserId(targetUserId)
	if not target or target == attacker or not DataService.isLoaded(target) then return end
	local vdata = DataService.get(target)
	if not vdata or vdata.Peaceful or byTarget[target] or byAttacker[target] then return end
	if DiveService.getState(attacker) == "Docked" or DiveService.getState(target) == "Docked" then return end
	if not inRange(attacker, target) then return end

	local t0 = os.time()
	if vdata.ShieldUntil > t0 then return end
	if t0 - vdata.LastRaidedAt < Raid.PostRaidProtection then return end
	local last = vdata.RaidedBy[tostring(attacker.UserId)]
	if last and t0 - last < Raid.PerVictimCooldown then
		notify(attacker, "You raided that player recently.")
		return
	end
	if CargoService.getExposed(target) < MIN_EXPOSED then return end

	if DefenseService.tryMine(target) then
		lockedUntil[attacker] = now + MINE_JAM_SECONDS
		notify(attacker, "Sonar Mine jammed your lock!")
		return
	end
	if math.random() < DefenseService.getDecoyChance(target) then
		lockedUntil[attacker] = now + DECOY_LOCKOUT
		notify(attacker, "You locked onto a Decoy Buoy!")
		return
	end

	local t: Tether = { attacker = attacker, target = target, startedAt = now, cancelled = false }
	byAttacker[attacker] = t
	byTarget[target] = t
	Remotes.event("RaidWarning"):FireClient(target, {
		attackerUserId = attacker.UserId, secondsLeft = Raid.TetherHoldSeconds,
	})
	task.spawn(holdLoop, t, target)
end

local function onCut(player: Player)
	local now = os.clock()
	if now - (lastCut[player] or 0) < 0.25 then return end
	lastCut[player] = now
	local t = byTarget[player] or byAttacker[player]
	if not t or now - t.startedAt < MIN_CUT_HOLD then return end
	local wasDrill = t.target == nil
	finish(t)
	if wasDrill then
		notify(player, "Nice cut! Drill complete.")
		TutorialService.advance(player, "TetherDrill")
	else
		notify(t.attacker, "Tether was cut.")
		if t.target then notify(t.target, "You cut the tether.") end
	end
end

local function onPeaceful(player: Player, on: any)
	if type(on) ~= "boolean" then return end
	local now = os.clock()
	if now - (lastToggle[player] or -TOGGLE_COOLDOWN) < TOGGLE_COOLDOWN then
		notify(player, "Wait before toggling Peaceful again.")
		return
	end
	if byAttacker[player] or byTarget[player] then return end
	local data = DataService.get(player)
	if not data or not DataService.isLoaded(player) then return end
	lastToggle[player] = now
	data.Peaceful = on
	notify(player, if on then "Peaceful mode on. You cannot raid or be raided." else "Peaceful mode off.")
end

function TetherRaidService.init()
	table.insert(conns, Remotes.event("TetherFire").OnServerEvent:Connect(onFire))
	table.insert(conns, Remotes.event("TetherCut").OnServerEvent:Connect(onCut))
	table.insert(conns, Remotes.event("PeacefulToggle").OnServerEvent:Connect(onPeaceful))
	table.insert(conns, Players.PlayerRemoving:Connect(function(p)
		local t = byAttacker[p] or byTarget[p]
		if t then finish(t) end
		lockedUntil[p] = nil; lastFire[p] = nil; lastCut[p] = nil; lastToggle[p] = nil
	end))
end

return TetherRaidService
