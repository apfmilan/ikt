# Game 7: CROSSWIND (Physics Glider Racing, Shared SkyPort Asset Suite)

Currencies: **Feathers** (soft), **Gems** (premium), **Wind Crystals** (special, from race podiums and events).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- **SkyPort Asset Suite** (shared package, reusable across other games): `SkyPortKit/{HangarModel, LaunchTower, WindTunnel, Terminal, PodiumStage, CosmeticRig}`. Published as a Wally package so any other game in the portfolio drops it in unchanged.
- `shared/GliderStats` (fixed classes: Light, Balanced, Heavy; identical for all players of a class), `shared/WindField` (deterministic wind vectors by seed).
- `server/Services/RaceService` (lobby, countdown, results), `GliderPhysicsValidator`, `WindService`, `HangarService`, `CrystalRaidService`, `HangarDefenseService`, `StormFrontEvent`.
- `client/Controllers/GliderController` (local physics for responsiveness), `RaceHUD`, `HangarUI`.
- State machine: `Lobby -> Countdown -> Racing -> Results`.

### 24-player capacity
- All 24 race in one shared airspace each round; client-owned glider physics, server validates speed/altitude bounds and checkpoints. Ghost-collision between gliders is off, but slipstream is on (shared skill mechanic).
- Remote updates at 10 Hz per glider with delta compression; client interpolates others. Courses stream in segments.
- Lobby fills from MemoryStore queue; between rounds a 20s SkyPort hangar phase lets players browse the shared hub.

### Competitive: Hangar Runs
- Wind Crystals earned in races sit in your **Hangar Vault**. Each crystal batch above the vault's Secured cap is Exposed.
- Rivals can run a Hangar Run: a short time-attack through your hangar's obstacle corridor; fastest clean run lifts up to 15% of Exposed crystals. Cooldown 10 min per victim, 50% refund; offline hangars locked.
- Glider performance is never affected by anything bought with Robux.

### Base defense (Feathers only)
| Structure | Effect |
|---|---|
| Vault Reinforcement | Raises Secured cap |
| Wind Gate | Adds a gust obstacle to the corridor |
| Laser Net | Adds a timing hazard |
| Sentry Falcon | NPC blocks one run per hour |
| Alarm Beacon | Alerts owner and marks the raider |

### Community event: Storm Front Race (every 10-15 min)
60s warning, then a server-wide race: all online players glide a storm-front course in one heat. Top 3 get Wind Crystals plus a lightning trail; everyone finishing gets Feathers and small Gems. Late joiners can enter for 30s. Course and wind are the same for everyone.

### Interactive tutorial
1. Launch from the tower. 2. Hit three wind rings. 3. Use slipstream behind a ghost glider. 4. Finish a short race. 5. Enter your Hangar and see the Vault. 6. Place a Wind Gate. 7. Currency Crash Course (Feathers / Gems / Wind Crystals, optional shop). 8. Reward: 25 Gems + 1 Wind Crystal.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "Sky Captain VIP", Description = "Cloud-white VIP tag and a golden contrail on your glider.", PriceRobux = 249, Type = "VIP" },
		{ Key = "WingSkins", Id = 0, Name = "Wing Skin Collection", Description = "Ten cosmetic wing designs. Aerodynamics are unchanged.", PriceRobux = 399, Type = "Cosmetic" },
		{ Key = "RaceReplay", Id = 0, Name = "Replay Studio", Description = "Save and replay your best races with a free camera.", PriceRobux = 199, Type = "QoL" },
		{ Key = "HudThemes", Id = 0, Name = "HUD Theme Pack", Description = "Custom HUD colors and layouts. No information advantage.", PriceRobux = 99, Type = "QoL" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "StormKey", Id = 0, Name = "Storm Front Key", Description = "Unlocks the bonus cosmetic reward track for one Storm Front. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Trophy Display Case", Description = "A showcase in your hangar for race trophies.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "Glider Design Slot", Description = "Save one more cosmetic glider design. No stats included.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts

**Icon (512x512, `--ar 1:1`):** Stylized Roblox 3D character in goggles on a colorful hang glider diving between glowing wind rings above puffy clouds, bright sunrise sky with a floating island skyport in the distance, bold outlines, saturated cyan, orange and pink palette, strong center focus, no text.

**Thumbnail (1920x1080, `--ar 16:9`):** Wide cinematic Roblox-style shot of 12 colorful gliders racing through giant storm clouds with lightning and glowing wind rings, a floating SkyPort with hangars and launch towers below, one glider leaving a golden contrail, dramatic sky gradient, space on the right for the title "CROSSWIND", thrilling speed mood.

## 4. Video Prompts (9:16)

**Video 1:** Vertical 9:16 Roblox 3D stylized look. Launch from a tower: camera follows over the glider's shoulder as it drops off the platform, wings snap open, then a rapid dive through three glowing rings, clouds streaking past, final pull-up into golden sunlight. Strong motion blur, rushing wind feel.

**Video 2:** Vertical 9:16 Roblox plastic 3D style. Packed race pack of gliders sliding into a slipstream behind the leader, lightning strikes behind them, camera whips alongside, a rival veers through a wind gate and boosts to the lead, confetti burst at the finish. Fast cuts, vibrant colors.

## 5. Voiceover Script (15s)

Voice: hyped sports commentator. Music: fast electronic with whoosh hits.

- 0:00-0:03 `[SFX: wind whoosh, countdown beep]` "Three, two, one, glide!"
- 0:03-0:07 `[SFX: ring chime, boost]` "Catch the wind. Ride the slipstream. Beat 23 rivals!"
- 0:07-0:11 `[SFX: thunder crack]` "Storm Front races hit every few minutes. Are you ready?"
- 0:11-0:15 `[SFX: whoosh out]` "Crosswind. Play free on Roblox!"
