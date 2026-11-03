# Game 4: FAULT LINE (Competitive City-Builder with Earthquakes)

Currencies: **Credits** (soft), **Gems** (premium), **Blueprint Tokens** (special, from event placements and landmarks).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- `shared/BuildingDefs` (cost, income, weight, stability rating), `shared/QuakeModel` (seeded, deterministic, same inputs = same damage).
- `server/Services/PlotService` (24 plots), `BuildService` (grid placement validation), `IncomeService`, `QuakeService`, `ConvoyRaidService`, `DefenseService`, `QuakeEvent`.
- `client/Controllers/BuildController` (grid ghost placement), `CityHUD`, `QuakeFXController` (camera shake, debris).
- State machine per round: `Build -> Tremor -> Quake -> Recovery`.

### 24-player capacity
- 24 plots on a shared fault-line map, 4 districts of 6 plots. Each plot has a building cap (max 120 parts) to protect performance.
- Quake damage is computed server-side from the seed and applied as batched attribute updates; clients play FX locally. Collapse physics are cosmetic debris pooled at max 200 parts.
- Neighbor plots share a fault segment, so decisions affect adjacent players (strategy, not P2W).

### Competitive: Supply Convoy raids
- Each city exports goods via a **Convoy** that drives along a road to the Port (Exposed while on the road, Secured at the Port).
- A rival can send a Raider Truck to ram a Convoy off the road (physics skill) and loot up to 15% of its load. Cooldown 10 min per victim, 50% refund. Cities with active protection or offline cities cannot be targeted.
- Blueprint Tokens can never be stolen.

### Base defense (Credits only)
| Structure | Effect |
|---|---|
| Seismic Damper | Reduces quake damage for the building it supports (tiered, hard cap) |
| Reinforced Foundation | Raises building stability rating |
| Road Barrier / Checkpoint | Slows raider trucks on your road |
| Fire Brigade | Cleans up post-quake fire damage faster |
| Guard Patrol | Auto-pursues raider trucks in your district |

All defenses cost Credits only. Gems cannot buy dampers, foundations or any safety margin.

### Community event: Great Quake (every 10-15 min)
60s siren warning, epicenter revealed on the map. Quake hits for 20s. Scores rank cities by **Stability Score** (buildings standing, value retained) computed server-side. Top 3 get Blueprint Tokens plus a landmark skin; everyone over threshold gets Credits and small Gems. Cities rebuild free after the event.

### Interactive tutorial
1. Place a house and a power plant. 2. Connect a road. 3. Send your first Convoy. 4. Place a Seismic Damper and see its effect in a gentle test tremor. 5. Defend a practice convoy raid. 6. Currency Crash Course (Credits / Gems / Blueprint Tokens, optional shop). 7. Reward: 25 Gems + 1 Blueprint Token.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "Mayor VIP", Description = "Mayor sash VIP tag and a golden city banner visible to neighbors.", PriceRobux = 249, Type = "VIP" },
		{ Key = "SkylineSkins", Id = 0, Name = "Skyline Skin Pack", Description = "Cosmetic building skins: art deco, neon and futuristic styles.", PriceRobux = 399, Type = "Cosmetic" },
		{ Key = "BuildTools", Id = 0, Name = "Pro Build Tools", Description = "Copy-paste and mirror placement. No stat or income change.", PriceRobux = 249, Type = "QoL" },
		{ Key = "CityPhotoMode", Id = 0, Name = "Photo Mode", Description = "Free camera and filters for city screenshots.", PriceRobux = 99, Type = "QoL" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "QuakeKey", Id = 0, Name = "Great Quake Key", Description = "Unlocks the bonus cosmetic reward track for one Quake. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Landmark Display Case", Description = "A showcase plaza for your earned landmarks and trophies.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "City Blueprint Slot", Description = "Save one more city layout design for reuse. Does not unlock new buildings.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts

**Icon (512x512, `--ar 1:1`):** Stylized Roblox 3D colorful mini city on a cracking ground, glowing orange fault line splitting the street, skyscrapers tilting, a hero damper tower standing firm in the center, dust and sparks, bold outlines, bright saturated palette, strong center focus, no text.

**Thumbnail (1920x1080, `--ar 16:9`):** Epic wide Roblox-style scene of a block city during an earthquake, ground cracking, buildings swaying, a siren glowing red, rival delivery trucks racing along the roads, one neighborhood protected by glowing blue seismic dampers, golden sunset, space on the right for the title "FAULT LINE", dramatic disaster-adventure mood, family-friendly.

## 4. Video Prompts (9:16)

**Video 1:** Vertical 9:16, Roblox 3D stylized look. Top-down camera swoops over a lively block city, then the ground trembles, camera shakes, a crack races across the map and buildings sway; one tower with a glowing damper barely moves while neighbors crumble in puffs of dust. Hard cut to a leaderboard with confetti.

**Video 2:** Vertical 9:16, Roblox plastic 3D style. A supply convoy truck speeds along a city road, a rival raider truck swerves in from the side and bumps it, crates scatter, a guard patrol siren lights up and chases. Fast low-angle tracking shot, motion blur, vibrant colors.

## 5. Voiceover Script (15s)

Voice: confident, urgent announcer. Music: rumbling bass and rising synth.

- 0:00-0:03 `[SFX: rumble, siren]` "Build your city. Beat the quake."
- 0:03-0:07 `[SFX: truck engine, crash]` "Protect your convoys from rivals on the road."
- 0:07-0:11 `[SFX: ground crack, bass drop]` "When the Great Quake hits, only the strongest cities stand!"
- 0:11-0:15 `[SFX: crowd cheer]` "Fault Line. Play free on Roblox!"
