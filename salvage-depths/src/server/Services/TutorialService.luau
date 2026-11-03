--!strict
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Shared = ReplicatedStorage:WaitForChild("Shared")
local Remotes = require(Shared:WaitForChild("Remotes"))
local GameConfig = require(Shared:WaitForChild("GameConfig"))
local DataService = require(script.Parent.DataService)
local EconomyService = require(script.Parent.EconomyService)

local STEPS: { { id: string, text: string } } = {
	{ id = "Dive", text = "Leave the dock and dive to 20m." },
	{ id = "Salvage", text = "Salvage a training wreck." },
	{ id = "Dock", text = "Return to dock: Exposed cargo becomes Secured." },
	{ id = "TetherDrill", text = "Try the safe tether-raid drill on the dummy sub. You can opt out with the Peaceful toggle." },
	{ id = "PlaceDefense", text = "Place a Decoy Buoy using Scrap." },
	{ id = "CurrencyCrashCourse", text = "Scrap: play loop. Gems: daily crate, milestones, events. Artifacts: rare objectives and event top 3. Gems never buy defenses or raid power. Shop is optional." },
	{ id = "Complete", text = "Tutorial complete! Rewards granted." },
}
local COMPLETE = #STEPS
local indexById: { [string]: number } = {}
for i, s in STEPS do indexById[s.id] = i end

local ACK_COOLDOWN = 0.5

local TutorialService = {}
local lastAck: { [Player]: number } = {}

-- TutorialStep persisted as current step index (0 = unstarted, treated as 1).
local function currentIndex(player: Player): number?
	local d = DataService.get(player)
	if not d then return nil end
	return math.clamp(d.TutorialStep, 1, COMPLETE)
end

local function send(player: Player, idx: number)
	if player.Parent ~= Players then return end
	Remotes.event("TutorialStep"):FireClient(player, { step = idx, id = STEPS[idx].id, text = STEPS[idx].text })
end

function TutorialService.getStep(player: Player): string?
	local i = currentIndex(player)
	return i and STEPS[i].id
end

function TutorialService.isComplete(player: Player): boolean
	return currentIndex(player) == COMPLETE
end

-- Called by other services on observed game events. Only advances if stepId is the current step.
function TutorialService.advance(player: Player, stepId: string): boolean
	if type(stepId) ~= "string" then return false end
	local d = DataService.get(player)
	local cur = currentIndex(player)
	if not d or not cur or cur >= COMPLETE or STEPS[cur].id ~= stepId then return false end
	local nextIdx = cur + 1
	d.TutorialStep = nextIdx
	if nextIdx == COMPLETE then
		EconomyService.addGems(player, GameConfig.Tutorial.RewardGems)
		EconomyService.addSpecial(player, GameConfig.Tutorial.RewardSpecial)
	end
	send(player, nextIdx)
	return true
end

function TutorialService.init()
	DataService.onLoaded(function(player)
		local d = DataService.get(player)
		if d and d.TutorialStep < 1 then d.TutorialStep = 1 end
		local i = currentIndex(player)
		if i then send(player, i) end
	end)
	Players.PlayerRemoving:Connect(function(p) lastAck[p] = nil end)

	-- Client may only acknowledge the info-only CurrencyCrashCourse panel.
	Remotes.event("TutorialAck").OnServerEvent:Connect(function(player: Player, stepId: any)
		local now = os.clock()
		if now - (lastAck[player] or -math.huge) < ACK_COOLDOWN then return end
		lastAck[player] = now
		if type(stepId) ~= "string" or #stepId > 32 or stepId ~= "CurrencyCrashCourse" then return end
		TutorialService.advance(player, stepId)
	end)
end

return TutorialService
