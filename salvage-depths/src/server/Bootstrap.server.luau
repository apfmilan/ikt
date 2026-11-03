--!strict
-- Server entry point.
-- NOTE: Players.MaxPlayers is a PLACE SETTING (Studio: Home > Game Settings > Players),
-- not settable from script. Set it to GameConfig.MaxPlayers (24) there.
local ReplicatedStorage = game:GetService("ReplicatedStorage")

-- Requiring Remotes on the server creates every RemoteEvent/RemoteFunction. Must be first.
require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))

local SERVICE_ORDER = {
	"DataService", "EconomyService", "MonetizationService", "TutorialService",
	"SectorService", "DiveService", "CargoService", "SalvageService",
	"DefenseService", "TetherRaidService", "EventScheduler",
}

local servicesFolder = script.Parent:WaitForChild("Services")

for _, name in SERVICE_ORDER do
	local module = servicesFolder:FindFirstChild(name)
	if not module or not module:IsA("ModuleScript") then
		if name == "DataService" then error("[Bootstrap] DataService missing; aborting boot") end
		warn(`[Bootstrap] service missing: {name}`)
		continue
	end
	local ok, service = pcall(require, module)
	if not ok then
		if name == "DataService" then error(`[Bootstrap] DataService failed to require: {service}`) end
		warn(`[Bootstrap] failed to require {name}: {service}`)
		continue
	end
	local svc = service :: any
	if type(svc) == "table" and type(svc.init) == "function" then
		local initOk, err = pcall(svc.init)
		if not initOk then
			if name == "DataService" then error(`[Bootstrap] DataService.init failed: {err}`) end
			warn(`[Bootstrap] {name}.init failed: {err}`)
		end
	else
		if name == "DataService" then error("[Bootstrap] DataService has no init()") end
		warn(`[Bootstrap] {name} has no init()`)
	end
end
