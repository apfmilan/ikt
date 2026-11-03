# Shared Core (applies to Games 1-8)

Every game file references this. Game files only list what is specific to that game.

## Fair-play rules (hard constraints)

- **Soft currency**: earned in play. Buys defenses, gear, upgrades.
- **Gems (premium)**: earned in play (daily, events, milestones) or bought. Buys cosmetics, display cases, loadout/blueprint slots, event cosmetic keys. Gems can NEVER buy defenses, raid power, shield time, protection, penalty/cooldown skips or safety margins.
- **Special currency (Artifacts / Blueprint Tokens / etc.)**: earned only through play. Never sold.
- Raids are skill-based. No Robux item changes raid success, loss caps or protection.
- Event keys unlock a bonus cosmetic reward track only. Every player competes in the same event under identical rules; free entry always exists.

## Rojo layout

```json
{
  "name": "Game",
  "tree": {
    "$className": "DataModel",
    "ReplicatedStorage": { "Shared": { "$path": "src/shared" } },
    "ServerScriptService": { "Server": { "$path": "src/server" } },
    "StarterPlayer": { "StarterPlayerScripts": { "Client": { "$path": "src/client" } } }
  }
}
```

```
src/shared/   Types.luau  Remotes.luau  MonetizationTypes.luau  GameConfig.luau  StateMachine.luau
src/server/   Bootstrap.server.luau
              Services/DataService  MatchService  EconomyService  RaidService  DefenseService
                       EventScheduler  TutorialService  MonetizationService  LeaderboardService
src/client/   Controllers/{HUD,Tutorial,Shop,Event,Raid}Controller   UI/
```

Packages (Wally): ProfileService, Promise, TestEZ. Strict typing (`--!strict`) everywhere.

## DataService (ProfileService)

```lua
--!strict
local Players = game:GetService("Players")
local ProfileService = require(script.Parent.Parent.Packages.ProfileService)

local TEMPLATE = {
	Version = 1,
	Soft = 0, Gems = 0, Special = 0,
	Receipts = {} :: { [string]: boolean },
	TutorialStep = 0,
	Owned = {} :: { [string]: boolean },
	LastRaidedAt = 0, ShieldUntil = 0,
}

local Store = ProfileService.GetProfileStore("PlayerData_v1", TEMPLATE)
local Profiles: { [Player]: any } = {}
local DataService = {}

function DataService.load(player: Player)
	local profile = Store:LoadProfileAsync("Player_" .. player.UserId)
	if not profile then player:Kick("Data load failed. Rejoin.") return end
	profile:AddUserId(player.UserId)
	profile:Reconcile()
	profile:ListenToRelease(function() Profiles[player] = nil; player:Kick() end)
	if player:IsDescendantOf(Players) then Profiles[player] = profile else profile:Release() end
end

function DataService.get(player: Player) return Profiles[player] and Profiles[player].Data end
function DataService.release(player: Player) local p = Profiles[player]; if p then p:Release() end end
return DataService
```

## StateMachine (match/round/event flow)

```lua
--!strict
export type State = { name: string, onEnter: (() -> ())?, onExit: (() -> ())? }
local StateMachine = {}
StateMachine.__index = StateMachine

function StateMachine.new(states: { [string]: State }, initial: string)
	local self = setmetatable({ states = states, current = initial }, StateMachine)
	local s = states[initial]; if s.onEnter then s.onEnter() end
	return self
end

function StateMachine:go(nextName: string)
	local old, new = self.states[self.current], self.states[nextName]
	assert(new, "Unknown state " .. nextName)
	if old.onExit then old.onExit() end
	self.current = nextName
	if new.onEnter then new.onEnter() end
end
return StateMachine
```

## EventScheduler (10-15 minute community events)

```lua
--!strict
local EventScheduler = {}
local MIN_DELAY, MAX_DELAY = 600, 900 -- seconds
local WARN_TIME = 60

function EventScheduler.start(events: { { name: string, run: (scores: { [Player]: number }) -> () }}, announce: (string, number) -> ())
	local rng = Random.new()
	task.spawn(function()
		while true do
			local delay = rng:NextInteger(MIN_DELAY, MAX_DELAY)
			task.wait(delay - WARN_TIME)
			local ev = events[rng:NextInteger(1, #events)]
			announce(ev.name, WARN_TIME) -- 60s warning banner
			task.wait(WARN_TIME)
			ev.run({}) -- each event module awards top-N + participation threshold
		end
	end)
end
return EventScheduler
```

Reward rule for every event: top 3 get special currency + exclusive cosmetic; everyone above a participation threshold gets soft currency + small Gems (5-15). Scores are server-computed only.

## 24-player scaling (common recipe)

1. Place setting `MaxPlayers = 24`; 24 pre-built plots/slots/spawn bays indexed 1-24.
2. `StreamingEnabled = true`; spawn world objects from a capped pool (live-instance budget per game file).
3. Replicate state via Attributes and batched remotes at 2-5 Hz, not per frame. Rate-limit every remote server-side.
4. MatchService fills servers via MemoryStoreService queue; friends join by `TeleportService`; crews use reserved servers.
5. Cross-server leaderboards: OrderedDataStore, written once per event end.
6. Heavy simulation (physics, AI) runs server-side on a fixed 10-20 Hz budget; client interpolates.

## Raid fairness constants (RaidService defaults)

| Rule | Value |
|---|---|
| Max loss per raid | 15% of exposed (unsecured) resources only |
| Per-attacker cooldown on same victim | 10 min |
| Free post-raid protection | 15 min, auto-granted |
| New-player shield | 30 min, free |
| Offline players | Never raidable |
| Victim compensation | 50% of loss refunded automatically |
| Opt-out | "Peaceful" toggle during tutorial |

## Tutorial framework (TutorialService)

Data-driven step list. Server validates each step by observed game events, never by client claim. Saved in `TutorialStep` so rejoin resumes. Final step is always the **Currency Crash Course**:

1. Panel "How you earn" with three live tracks: Soft (play loop), Gems (daily crate, milestones, events), Special (rare objectives, event top-3).
2. Player performs one earn action for each track (guided, gets a small starter reward).
3. "Optional: Shop" panel shows Gem packs and Game Passes neutrally, skippable, never blocks progress. Text states plainly: "Gems never buy defenses or raid power."
4. Completion grants 25 Gems + 1 Special currency.

## MonetizationService skeleton

```lua
--!strict
local MarketplaceService = game:GetService("MarketplaceService")
local DataService = require(script.Parent.DataService)
local Catalog = require(game.ReplicatedStorage.Shared.GameConfig).Monetization

MarketplaceService.ProcessReceipt = function(info)
	local player = game.Players:GetPlayerByUserId(info.PlayerId)
	if not player then return Enum.ProductPurchaseDecision.NotProcessedYet end
	local data = DataService.get(player)
	if not data then return Enum.ProductPurchaseDecision.NotProcessedYet end
	local key = tostring(info.PurchaseId)
	if data.Receipts[key] then return Enum.ProductPurchaseDecision.PurchaseGranted end
	local def = Catalog.byProductId[info.ProductId]
	if not def then return Enum.ProductPurchaseDecision.NotProcessedYet end
	if def.Grant.Gems then data.Gems += def.Grant.Gems end
	data.Receipts[key] = true
	return Enum.ProductPurchaseDecision.PurchaseGranted
end
```

## Shared monetization types

```lua
--!strict
export type ItemType = "Cosmetic" | "VIP" | "QoL" | "Currency" | "EventKey" | "DisplayCase" | "BlueprintSlot"
export type Def = {
	Key: string, Id: number, Name: string, Description: string,
	PriceRobux: number, Type: ItemType, Grant: { [string]: any }?,
}
return {}
```

IDs are `0` placeholders; fill after creating items in Creator Hub.
