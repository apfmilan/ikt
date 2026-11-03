--!strict
-- Tunables. Fair-play constants mirror roblox-blueprints/00-shared-core.md.
local GameConfig = {}

GameConfig.MaxPlayers = 24
GameConfig.Raid = {
	MaxLossPct = 0.15,
	VictimRefundPct = 0.5,
	PerVictimCooldown = 600,
	PostRaidProtection = 900,
	NewPlayerShield = 1800,
	TetherRange = 60,
	TetherHoldSeconds = 8,
}
GameConfig.Event = { MinDelay = 600, MaxDelay = 900, WarnSeconds = 60, DurationSeconds = 180, ParticipationThreshold = 1 }
GameConfig.Dive = { MaxOxygen = 100, OxygenPerSecondPerMeter = 0.01, BaseMaxDepth = 60, DepthPerPlateTier = 40 }
GameConfig.Sectors = { Count = 6, BaysPerSector = 4, WreckPoolCap = 80, WreckRespawnSeconds = 90 }
GameConfig.DefenseDefs = {
	DockVault   = { MaxTier = 5, CostPerTier = { 150, 400, 900, 2000, 4500 } },
	DecoyBuoy   = { MaxTier = 3, CostPerTier = { 200, 600, 1500 } },
	SonarMine   = { MaxTier = 3, CostPerTier = { 300, 800, 2000 } },
	GuardDrone  = { MaxTier = 2, CostPerTier = { 1200, 3500 } },
	HullPlating = { MaxTier = 5, CostPerTier = { 100, 300, 800, 1800, 4000 } },
}
GameConfig.Tutorial = { RewardGems = 25, RewardSpecial = 1 }
GameConfig.Events = { "TrenchSurge" }

return GameConfig
