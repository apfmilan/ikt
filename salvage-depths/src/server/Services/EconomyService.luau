--!strict
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Shared = ReplicatedStorage:WaitForChild("Shared")
local Remotes = require(Shared:WaitForChild("Remotes"))
local Types = require(Shared:WaitForChild("Types"))
local DataService = require(script.Parent.DataService)

local MAX_AMOUNT = 1e9
local MAX_BALANCE = 2e9
local SNAPSHOT_COOLDOWN = 1

local EconomyService = {}
local lastSnapshot: { [Player]: number } = {}

local function validAmount(n: any): boolean
	return type(n) == "number" and n == n and n > 0 and n <= MAX_AMOUNT and n == math.floor(n)
end

function EconomyService.getBalance(player: Player): Types.Balance
	local d = DataService.get(player)
	if not d then return { soft = 0, gems = 0, special = 0 } end
	return { soft = d.Soft, gems = d.Gems, special = d.Special }
end

local function push(player: Player)
	if player.Parent ~= Players then return end
	Remotes.event("BalanceUpdate"):FireClient(player, EconomyService.getBalance(player))
end

local function add(player: Player, field: "Soft" | "Gems" | "Special", n: number)
	if not validAmount(n) then return end
	local d = DataService.get(player)
	if not d then return end
	d[field] = math.min(d[field] + n, MAX_BALANCE)
	push(player)
end

local function spend(player: Player, field: "Soft" | "Gems", n: number): boolean
	if not validAmount(n) then return false end
	local d = DataService.get(player)
	if not d or d[field] < n then return false end
	d[field] -= n
	push(player)
	return true
end

function EconomyService.addSoft(player: Player, n: number) add(player, "Soft", n) end
function EconomyService.addGems(player: Player, n: number) add(player, "Gems", n) end
function EconomyService.addSpecial(player: Player, n: number) add(player, "Special", n) end
function EconomyService.spendSoft(player: Player, n: number): boolean return spend(player, "Soft", n) end
-- Gems: cosmetics / slots / display cases / event keys ONLY. Never defenses, raid power, shields.
function EconomyService.spendGems(player: Player, n: number): boolean return spend(player, "Gems", n) end

function EconomyService.init()
	DataService.onLoaded(push)
	Players.PlayerRemoving:Connect(function(p) lastSnapshot[p] = nil end)

	Remotes.func("GetProfileSnapshot").OnServerInvoke = function(player: Player)
		local now = os.clock()
		if now - (lastSnapshot[player] or -math.huge) < SNAPSHOT_COOLDOWN then return nil end
		lastSnapshot[player] = now
		local d = DataService.get(player)
		if not d then return nil end
		local defenses: { [string]: number } = {}
		for k, v in d.Defenses do defenses[k] = v end
		return {
			soft = d.Soft, gems = d.Gems, special = d.Special,
			tutorialStep = d.TutorialStep, defenses = defenses,
		}
	end
end

return EconomyService
