--!strict
-- Exposed hold (raidable, in-memory) vs Secured vault (profile.SecuredCargo, never raidable).
-- Docking cashes ALL Exposed units in as Scrap (soft); the Secured counter is a display total capped at Dock Vault capacity,
-- so income never stalls at capacity.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Remotes = require(ReplicatedStorage.Shared.Remotes)
local DataService = require(script.Parent.DataService)
local EconomyService = require(script.Parent.EconomyService)
local TutorialService = require(script.Parent.TutorialService)
local DefenseService = require(script.Parent.DefenseService)
local DiveService = require(script.Parent.DiveService)

local BASE_CAPACITY = 400
local CAPACITY_PER_TIER = 400

local CargoService = {}
local exposed: { [Player]: number } = {}
local lastReq: { [Player]: number } = {}
local conns: { RBXScriptConnection } = {}

function CargoService.getCapacity(player: Player): number
	return BASE_CAPACITY + CAPACITY_PER_TIER * DefenseService.getTier(player, "DockVault")
end

function CargoService.getExposed(player: Player): number
	return exposed[player] or 0
end

local function push(player: Player)
	local data = DataService.get(player)
	Remotes.event("CargoUpdate"):FireClient(player, {
		exposed = exposed[player] or 0,
		secured = if data then data.SecuredCargo else 0,
		capacity = CargoService.getCapacity(player),
	})
end

function CargoService.addExposed(player: Player, n: number)
	if n ~= n or n <= 0 or not player.Parent then return end
	exposed[player] = (exposed[player] or 0) + math.floor(n)
	push(player)
end

-- Removes up to n units; returns amount actually taken.
function CargoService.takeExposed(player: Player, n: number): number
	if n ~= n or n <= 0 then return 0 end
	local have = exposed[player] or 0
	local taken = math.min(have, math.floor(n))
	if taken <= 0 then return 0 end
	exposed[player] = have - taken
	push(player)
	return taken
end

-- Cashes out the whole Exposed hold; returns units moved.
local function settle(player: Player): number
	local data = DataService.get(player)
	if not data or not DataService.isLoaded(player) then return 0 end
	local moved = CargoService.takeExposed(player, exposed[player] or 0)
	if moved > 0 then
		data.SecuredCargo = math.min(CargoService.getCapacity(player), data.SecuredCargo + moved)
		EconomyService.addSoft(player, moved)
	end
	push(player)
	return moved
end

local function onDock(player: Player)
	local now = os.clock()
	if now - (lastReq[player] or 0) < 1 then return end
	lastReq[player] = now
	local data = DataService.get(player)
	if not data or not DataService.isLoaded(player) then return end
	if not DiveService.canDock(player) then
		Remotes.event("Notify"):FireClient(player, { text = "Return to the surface to dock." })
		return
	end
	local moved = settle(player)
	DiveService.dock(player)
	Remotes.event("Notify"):FireClient(player, { text = ("Docked. Cashed in %d units."):format(moved) })
	TutorialService.advance(player, "Dock")
end

function CargoService.init()
	table.insert(conns, Remotes.event("DockRequest").OnServerEvent:Connect(onDock))
	table.insert(conns, Players.PlayerRemoving:Connect(function(p)
		exposed[p] = nil; lastReq[p] = nil
	end))
	DataService.onLoaded(push)
	DiveService.onDocked(function(p: Player) settle(p) end) -- auto-dock (surfacing) must not leave a full exposed hold safe
	table.insert(conns, Remotes.event("ClientReady").OnServerEvent:Connect(push))
	DefenseService.onChanged(function(p, kind)
		if kind == "DockVault" then push(p) end
	end)
end

return CargoService
