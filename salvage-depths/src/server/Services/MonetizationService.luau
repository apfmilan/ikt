--!strict
local MarketplaceService = game:GetService("MarketplaceService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Shared = ReplicatedStorage:WaitForChild("Shared")
local Catalog = require(Shared:WaitForChild("Monetization"))
local Types = require(Shared:WaitForChild("Types"))
local DataService = require(script.Parent.DataService)
local EconomyService = require(script.Parent.EconomyService)

type Def = Types.MonetizationDef

local MonetizationService = {}

local NUMERIC_GRANTS = { EventKeys = true, DisplayCases = true, BlueprintSlots = true }

local function grantProduct(player: Player, def: Def): boolean
	local data = DataService.get(player)
	if not data then return false end
	local grant = def.Grant
	if not grant then return true end
	for k, v in grant do
		if type(v) ~= "number" or v <= 0 then continue end
		if k == "Gems" then
			EconomyService.addGems(player, v)
		elseif NUMERIC_GRANTS[k] then
			(data :: any)[k] += v
		end
	end
	return true
end

local function grantPass(player: Player, def: Def)
	local data = DataService.get(player)
	if data then data.GamePasses[def.Key] = true end
end

function MonetizationService.hasPass(player: Player, key: string): boolean
	local data = DataService.get(player)
	return data ~= nil and data.GamePasses[key] == true
end

local function checkOwnership(player: Player)
	for _, def in Catalog.GamePasses do
		if def.Id == 0 then continue end
		local ok, owns = pcall(MarketplaceService.UserOwnsGamePassAsync, MarketplaceService, player.UserId, def.Id)
		if ok and owns == true then
			grantPass(player, def)
		elseif not ok then
			warn("[Monetization] ownership check failed for " .. def.Key)
		end
	end
end

function MonetizationService.init()
	DataService.onLoaded(checkOwnership)

	MarketplaceService.ProcessReceipt = function(info)
		local player = game:GetService("Players"):GetPlayerByUserId(info.PlayerId)
		if not player then return Enum.ProductPurchaseDecision.NotProcessedYet end
		local data = DataService.get(player)
		if not data then return Enum.ProductPurchaseDecision.NotProcessedYet end
		local key = tostring(info.PurchaseId)
		if data.Receipts[key] then return Enum.ProductPurchaseDecision.PurchaseGranted end
		local def = Catalog.byProductId[info.ProductId]
		if not def then return Enum.ProductPurchaseDecision.NotProcessedYet end
		-- Mark first so a re-entrant call cannot double grant.
		data.Receipts[key] = true
		if not grantProduct(player, def) then
			data.Receipts[key] = nil
			return Enum.ProductPurchaseDecision.NotProcessedYet
		end
		return Enum.ProductPurchaseDecision.PurchaseGranted
	end

	MarketplaceService.PromptGamePassPurchaseFinished:Connect(function(player: Player, passId: number, wasPurchased: boolean)
		if not wasPurchased then return end
		local def = Catalog.byPassId[passId]
		if def then grantPass(player, def) end
	end)
end

return MonetizationService
