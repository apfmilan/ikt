--!strict
-- 6 sectors, capped wreck pool, seeded spawns, per-wreck respawn timer.
local CollectionService = game:GetService("CollectionService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local Workspace = game:GetService("Workspace")

local GameConfig = require(ReplicatedStorage.Shared.GameConfig)

export type Wreck = {
	id: string, part: Part, sector: number,
	value: number, depth: number, relic: boolean, claimed: boolean,
}

local SEED = 1337
local RING_RADIUS = 250
local SECTOR_SPREAD = 120
local SHALLOW_PER_SECTOR = 3

local SectorService = {}
local wrecks: { [string]: Wreck } = {}
local perSector: { number } = {}
local liveCount = 0
local nextId = 0
local rng = Random.new(SEED)
local folder: Folder

local function sectorCap(): number
	return math.floor(GameConfig.Sectors.WreckPoolCap / GameConfig.Sectors.Count)
end

function SectorService.getSectorCenter(sector: number): Vector3
	local a = (sector - 1) / GameConfig.Sectors.Count * math.pi * 2
	return Vector3.new(math.cos(a) * RING_RADIUS, 0, math.sin(a) * RING_RADIUS)
end

local function spawnWreck(sector: number, shallow: boolean)
	if liveCount >= GameConfig.Sectors.WreckPoolCap or perSector[sector] >= sectorCap() then return end
	nextId += 1
	local depth = if shallow then rng:NextInteger(10, 35) else math.floor(10 + rng:NextNumber() ^ 1.5 * 210)
	local relic = depth >= 100 and rng:NextNumber() < 0.25
	local value = math.floor(8 + depth * 0.6 * (0.7 + rng:NextNumber() * 0.6))
	if relic then value *= 2 end
	local center = SectorService.getSectorCenter(sector)
	local offset = Vector3.new(rng:NextNumber(-1, 1) * SECTOR_SPREAD, 0, rng:NextNumber(-1, 1) * SECTOR_SPREAD)

	local id = "W" .. tostring(nextId)
	local part = Instance.new("Part")
	part.Name = id
	part.Anchored = true
	part.Size = Vector3.new(rng:NextInteger(6, 14), rng:NextInteger(4, 8), rng:NextInteger(10, 22))
	part.Color = if relic then Color3.fromRGB(255, 205, 60) else Color3.fromRGB(95, 110, 120)
	part.Position = center + offset + Vector3.new(0, -depth, 0)
	part:SetAttribute("WreckId", id)
	part:SetAttribute("Value", value)
	part:SetAttribute("Depth", depth)
	part:SetAttribute("Relic", relic)
	CollectionService:AddTag(part, "Wreck")
	part.Parent = folder

	wrecks[id] = { id = id, part = part, sector = sector, value = value, depth = depth, relic = relic, claimed = false }
	perSector[sector] += 1
	liveCount += 1
end

function SectorService.getWreck(id: string): Wreck?
	local w = wrecks[id]
	if w and not w.claimed then return w end
	return nil
end

-- Atomically claims a wreck; returns its value or nil if gone/already claimed.
function SectorService.claimWreck(id: string): number?
	local w = wrecks[id]
	if not w or w.claimed then return nil end
	w.claimed = true
	wrecks[id] = nil
	perSector[w.sector] -= 1
	liveCount -= 1
	w.part:Destroy()
	task.delay(GameConfig.Sectors.WreckRespawnSeconds, function()
		spawnWreck(w.sector, false)
	end)
	return w.value
end

function SectorService.init()
	local existing = Workspace:FindFirstChild("Wrecks")
	if existing then existing:Destroy() end
	folder = Instance.new("Folder")
	folder.Name = "Wrecks"
	folder.Parent = Workspace
	for s = 1, GameConfig.Sectors.Count do
		perSector[s] = 0
		for i = 1, sectorCap() do
			spawnWreck(s, i <= SHALLOW_PER_SECTOR)
		end
	end
end

return SectorService
