--!strict
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local GameConfig = require(ReplicatedStorage.Shared.GameConfig)
local Remotes = require(ReplicatedStorage.Shared.Remotes)
local TrenchSurgeEvent = require(script.Parent.TrenchSurgeEvent)

local EVENTS: { [string]: () -> ({ [Player]: number }, boolean) } = {
	TrenchSurge = TrenchSurgeEvent.run,
}

local EventScheduler = {}
local running = false
local rng = Random.new()

local function runEvent(name: string, warnSeconds: number): boolean
	local run = EVENTS[name]
	if running or not run then return true end
	running = true
	local duration = GameConfig.Event.DurationSeconds
	Remotes.event("EventAnnounce"):FireAllClients({ name = name, startsIn = warnSeconds, duration = duration })
	task.wait(warnSeconds)
	Remotes.event("EventAnnounce"):FireAllClients({ name = name, startsIn = 0, duration = duration })
	local ok, res, sealed = pcall(run)
	if not ok then
		warn("[EventScheduler] event failed: " .. tostring(res))
		Remotes.event("EventAnnounce"):FireAllClients({ ended = true, sealed = false })
	end
	running = false
	return ok and sealed == true
end

local function pickName(): string
	local list = GameConfig.Events
	return list[rng:NextInteger(1, #list)]
end

-- QA: starts an event after a short warning. Returns false if one is already running.
function EventScheduler.triggerNow(): boolean
	if running then return false end
	task.spawn(runEvent, pickName(), 5)
	return true
end

function EventScheduler.init()
	local ev = GameConfig.Event
	task.spawn(function()
		while true do
			local delay = rng:NextInteger(ev.MinDelay, ev.MaxDelay)
			task.wait(math.max(0, delay - ev.WarnSeconds))
			if #Players:GetPlayers() > 0 and not running then
				local sealed = runEvent(pickName(), ev.WarnSeconds)
				if not sealed then task.wait(ev.MinDelay) end -- failed Seal delays next spawn
			end
		end
	end)
end

return EventScheduler
