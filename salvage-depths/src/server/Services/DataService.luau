--!strict
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")

local Types = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Types"))
local GameConfig = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("GameConfig"))
local ProfileService = require(ReplicatedStorage:WaitForChild("Packages"):WaitForChild("ProfileService")) :: any

type ProfileData = Types.ProfileData

local TEMPLATE: ProfileData = {
	Version = 2,
	Soft = 0, Gems = 0, Special = 0,
	Receipts = {},
	ReceiptOrder = {},
	GamePasses = {},
	TutorialStep = 0,
	Defenses = {},
	SecuredCargo = 0,
	LastRaidedAt = 0,
	ShieldUntil = 0,
	RaidedBy = {},
	EventKeys = 0, DisplayCases = 0, BlueprintSlots = 0,
	Peaceful = false,
}

local STORE_NAME = "SalvageDepths_PlayerData_v1"

local DataService = {}

local store: any = nil
local profiles: { [Player]: any } = {}
local callbacks: { (Player) -> () } = {}
local initialized = false
local loading: { [Player]: boolean } = {}

-- Runs after Reconcile. Each step upgrades data from Version N to N+1.
local function migrate(data: ProfileData)
	if data.Version < 2 then
		-- v2: ReceiptOrder added. Seed from existing receipts (order unknown).
		for key in data.Receipts do table.insert(data.ReceiptOrder, key) end
		data.Version = 2
	end
end

local function fire(cb: (Player) -> (), player: Player)
	task.spawn(function()
		local ok, err = pcall(cb, player)
		if not ok then warn("[DataService] onLoaded callback error: " .. tostring(err)) end
	end)
end

local function load(player: Player)
	if loading[player] or profiles[player] then return end
	loading[player] = true
	local ok, profile = pcall(store.LoadProfileAsync, store, "Player_" .. player.UserId, "ForceLoad")
	loading[player] = nil
	if not ok then warn("[DataService] LoadProfileAsync failed: " .. tostring(profile)) end
	if not ok or not profile then
		player:Kick("Data failed to load. Please rejoin.")
		return
	end
	profile:AddUserId(player.UserId)
	profile:Reconcile()
	migrate(profile.Data :: ProfileData)
	profile:ListenToRelease(function()
		profiles[player] = nil
		if player.Parent == Players then player:Kick("Data session ended. Please rejoin.") end
	end)
	if player.Parent ~= Players then
		profile:Release()
		return
	end
	local data: ProfileData = profile.Data
	if data.ShieldUntil == 0 then
		data.ShieldUntil = os.time() + GameConfig.Raid.NewPlayerShield
	end
	profiles[player] = profile
	for _, cb in callbacks do fire(cb, player) end
end

function DataService.init()
	if initialized then return end
	initialized = true
	store = ProfileService.GetProfileStore(STORE_NAME, TEMPLATE)
	Players.PlayerAdded:Connect(function(p) load(p) end)
	Players.PlayerRemoving:Connect(function(p) DataService.release(p) end)
	for _, p in Players:GetPlayers() do task.spawn(load, p) end
	game:BindToClose(function()
		for p in profiles do DataService.release(p) end
		local deadline = os.clock() + 25
		while next(profiles) ~= nil and os.clock() < deadline do
			task.wait(0.1)
		end
		if RunService:IsStudio() then task.wait(0.5) end
	end)
end

function DataService.get(player: Player): ProfileData?
	local p = profiles[player]
	return p and p.Data
end

function DataService.isLoaded(player: Player): boolean
	return profiles[player] ~= nil
end

-- Forces a save if the ProfileService build exposes profile:Save(). false = save failed.
function DataService.saveNow(player: Player): boolean
	local p = profiles[player]
	if not p then return false end
	if type(p.Save) ~= "function" then return true end
	local ok, err = pcall(p.Save, p)
	if not ok then warn("[DataService] Save failed: " .. tostring(err)) end
	return ok
end

function DataService.release(player: Player)
	local p = profiles[player]
	if p then
		profiles[player] = nil
		p:Release()
	end
end

-- Fires for players loaded later AND already-loaded players.
function DataService.onLoaded(callback: (Player) -> ())
	table.insert(callbacks, callback)
	for p in profiles do fire(callback, p) end
end

return DataService
