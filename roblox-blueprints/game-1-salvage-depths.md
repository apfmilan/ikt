# Game 1: SALVAGE DEPTHS (Submarine Scavenging)

Currencies: **Scrap** (soft), **Gems** (premium), **Artifacts** (special, from relic wrecks and events).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- `shared/WreckTable`, `shared/SubStats` (hull, oxygen, hold size, light radius).
- `server/Services/SectorService` (seeded wreck spawns), `DiveService` (oxygen, pressure, depth bands), `CargoService` (Secured vs Exposed hold), `SalvageService` (grab minigame), `TetherRaidService`, `SonarDefenseService`, `TrenchSurgeEvent`.
- `client/Controllers/DiveHUD`, `SonarController`, `CargoUI`.
- State machine per sub: `Docked -> Diving -> Salvaging -> Surfacing -> Docked`.

### 24-player capacity
- Ocean split into 6 sectors, 4 dock bays each (24 bays total). Wrecks spawn from a capped pool (max 80 live wreck models); respawn on a 90s timer per sector.
- Sonar pings replicate as Attributes at 2 Hz. Sub physics owned by each client (network ownership) with server sanity checks on speed and depth.
- Dense wrecks near center pull players into contact; deep relic wrecks need depth upgrades (earned), keeping the crowd spread.

### Competitive: Cargo Tether raids
- Items collected go into the **Exposed hold** until docked. Docking moves them to **Secured**.
- A rival fires a tether at an Exposed hold within 60 studs. Hold line 8s; the victim sees a warning and can cut the tether (hold E) or outrun it.
- Success steals up to 15% of Exposed Scrap/items (never Secured, never Artifacts). 10-min cooldown per victim; 50% refund to the victim.

### Base defense (Scrap only)
| Structure | Effect | Tier cap |
|---|---|---|
| Dock Vault | Larger Secured capacity | 5 |
| Decoy Buoy | Fake cargo signature on rival sonar | 3 |
| Sonar Mine | Jams tether lock for 5s (no damage) | 3 |
| Guard Drone | Auto-cuts a tether in range every 20s | 2 |
| Hull Plating | Deeper dive limit (PvE only) | 5 |

### Community event: Trench Surge (every 10-15 min)
60s warning banner, then 3 minutes: vents open across the map and the shared Pressure Meter rises. Players deliver Reactor Cores to the central Seal. If the meter is sealed in time, everyone gets a participation reward. Top 3 contributors get Artifacts + exclusive hull decal. If it fails, the next spawn is delayed (no penalty to players).

### Interactive tutorial
1. Leave dock and dive to 20m. 2. Salvage a training wreck. 3. Dock and see the Exposed to Secured transfer. 4. Safe tether-raid drill against a dummy sub (opt-out toggle explained). 5. Place a Decoy Buoy. 6. Currency Crash Course (Scrap / Gems / Artifacts earn tracks, optional shop peek). 7. Reward: 25 Gems + 1 Artifact.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "Deep Diver VIP", Description = "Gold VIP tag in chat, VIP dock banner and a unique sonar ping color.", PriceRobux = 249, Type = "VIP" },
		{ Key = "NeonHulls", Id = 0, Name = "Neon Hull Pack", Description = "Five glowing hull paint jobs and a bioluminescent trail.", PriceRobux = 399, Type = "Cosmetic" },
		{ Key = "AutoSort", Id = 0, Name = "Auto-Sort Cargo", Description = "Automatically sorts your hold by value and rarity. Quality of life only.", PriceRobux = 199, Type = "QoL" },
		{ Key = "ExtraLoadouts", Id = 0, Name = "Extra Loadout Slots", Description = "Save 5 more cosmetic and gear loadouts. No stat advantage.", PriceRobux = 149, Type = "QoL" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "SurgeKey", Id = 0, Name = "Trench Surge Key", Description = "Unlocks the bonus cosmetic reward track for one Surge. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Relic Display Case", Description = "A glass case in your dock to show off found Artifacts.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "Sub Blueprint Slot", Description = "One more slot to store sub designs you have already unlocked.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts (Midjourney / DALL-E 3)

**Icon (512x512, `--ar 1:1`):** Stylized low-poly Roblox-style yellow research submarine with glowing headlights diving toward a golden shipwreck on a deep teal seabed, bioluminescent jellyfish in the background, bold cartoon outline, saturated colors, strong center focus, no text, clean readable at small size, game icon composition.

**Thumbnail (1920x1080, `--ar 16:9`):** Epic wide shot in Roblox 3D style: three colorful mini submarines racing through a sunken city of shipwrecks and coral, a giant glowing relic orb in the center, rival sub firing a glowing tether at a cargo hold, dramatic volumetric light rays from the surface, deep blue and neon orange palette, empty space on the left for title text "SALVAGE DEPTHS", high contrast, cinematic.

## 4. Video Prompts (Runway Gen-3 / Luma, 9:16)

**Video 1:** Vertical 9:16, stylized Roblox 3D look. A yellow submarine plunges through dark water, headlights cutting through murk, camera follows from behind then swings to a front shot as a glowing golden wreck appears. Mechanical claw grabs a treasure chest, gold coins burst out, sonar ping rings pulse across the screen. Fast push-in, volumetric light, vibrant color, high energy.

**Video 2:** Vertical 9:16, Roblox plastic 3D style. Rival submarine fires a glowing blue tether at another sub's cargo hold; the victim sub boosts away, tether snaps with a flash. Quick whip-pan to a massive vent erupting in the trench with a red pressure gauge rising. Dynamic handheld camera shake, bubbles, dramatic lighting, fast cuts.

## 5. Voiceover Script (ElevenLabs, 15s)

Voice: energetic male, fast pace. Music: driving electronic.

- 0:00-0:03 `[SFX: sonar ping, deep whoosh]` "Dive deep. Grab the treasure."
- 0:03-0:07 `[SFX: tether zap, metal clank]` "Rivals are watching your cargo! Dock it before it's gone!"
- 0:07-0:11 `[SFX: alarm, bass drop]` "Trench Surge is rising. Team up and seal the vents!"
- 0:11-0:15 `[SFX: coin burst]` "Salvage Depths. Play free on Roblox now!"
