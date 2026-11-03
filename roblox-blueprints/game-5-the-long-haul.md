# Game 5: THE LONG HAUL (Solo & Convoy Radio Trucking)

Currencies: **Dollars** (soft), **Gems** (premium), **Haul Tokens** (special, from long routes and events).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- `shared/RouteGraph` (highways, depots, weather zones), `shared/CargoDefs` (value, fragility, weight), `shared/RadioChannels`.
- `server/Services/JobService` (contract board), `TruckService` (vehicle stats, fuel), `ConvoyService` (join, draft bonus), `RadioService` (text-filtered CB channels), `HeistService` (raids), `SecurityService`, `InterstateRushEvent`.
- `client/Controllers/DriveController`, `RadioUI`, `MapHUD`.
- State machine per job: `Idle -> Loaded -> InTransit -> Delivered | Failed`.

### 24-player capacity
- One large highway loop with 6 depots and 4 radio channels (6 players per channel by default). Streaming handles distant terrain; traffic NPCs capped at 30.
- Trucks use client network ownership; server validates speed and route progress checkpoints.
- Convoy bonus: up to 4 trucks per convoy; drafting gives small fuel saving to followers (shared mechanic, not purchasable).

### Competitive: Cargo Heist
- Trailer loads are **Exposed** while in transit and **Secured** at delivery. Delivering pays the contract.
- A bandit player can pull alongside and run a 6s "Trailer Pop" timing minigame to steal up to 15% of the load. Truck stops are safe zones (no heists). 10-min cooldown per victim, 50% refund. Offline players' parked trucks are untouchable.
- Convoys with 2+ trucks get an automatic 25% reduction in heist success (earned by playing together).

### Base defense (Dollars only)
| Structure | Effect |
|---|---|
| Trailer Lock | Heist minigame gets longer |
| GPS Tracker | Marks the bandit on map for 30s |
| Escort NPC | AI truck blocks one heist attempt per route |
| Radar Detector | Warns of bandits within 80 studs |
| Garage Safe | Stores Secured cargo bonus at the depot |

### Community event: Interstate Rush (every 10-15 min)
60s radio warning. A priority "Rush Load" appears at the central depot; deliver as many Rush crates as possible to the far depot within 4 minutes, avoiding a rolling blizzard zone. Top 3 deliverers get Haul Tokens plus a chrome truck decal; participants over threshold get Dollars and small Gems.

### Interactive tutorial
1. Pick a contract and hook a trailer. 2. Drive to a checkpoint. 3. Use the radio (preset phrases) to call a convoy buddy. 4. Experience a safe heist drill with an AI bandit. 5. Buy a Trailer Lock. 6. Deliver and get paid. 7. Currency Crash Course (Dollars / Gems / Haul Tokens, optional shop). 8. Reward: 25 Gems + 1 Haul Token.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "Road King VIP", Description = "Chrome VIP tag, VIP radio call sign color and a special horn.", PriceRobux = 249, Type = "VIP" },
		{ Key = "PaintShop", Id = 0, Name = "Custom Paint Shop", Description = "Unlimited custom truck paint, flames and decals. Cosmetic only.", PriceRobux = 399, Type = "Cosmetic" },
		{ Key = "CruiseAssist", Id = 0, Name = "Cruise Assist", Description = "Cruise control and lane-keep display. No speed or fuel advantage.", PriceRobux = 199, Type = "QoL" },
		{ Key = "RadioPresets", Id = 0, Name = "Radio Preset Pack", Description = "More radio preset phrases and call-sign styles.", PriceRobux = 99, Type = "QoL" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "RushKey", Id = 0, Name = "Interstate Rush Key", Description = "Unlocks the bonus cosmetic reward track for one Rush. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Trophy Display Case", Description = "A showcase shelf in your garage for earned trophies.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "Truck Build Slot", Description = "Save one more truck cosmetic build. No stats included.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts

**Icon (512x512, `--ar 1:1`):** Stylized Roblox 3D chrome semi-truck with glowing headlights barreling toward the camera down a neon sunset highway, radio antenna with sparkling signal waves, bold outlines, orange and purple palette, strong center focus, no text.

**Thumbnail (1920x1080, `--ar 16:9`):** Wide cinematic Roblox-style highway scene with a convoy of four colorful trucks at dusk, a bandit pickup pulling alongside with a glowing timing ring over a trailer, blizzard wall approaching on the horizon, radio waveform graphic in the sky, space on the left for the title "THE LONG HAUL", adventure and speed mood.

## 4. Video Prompts (9:16)

**Video 1:** Vertical 9:16 Roblox 3D stylized look. Low camera at asphalt level as a chrome truck roars past, camera whips to the cab interior with a glowing radio display crackling "Convoy, you copy?", then drone shot pulling back to four trucks in formation on a sunset highway. Warm light, motion blur, high energy.

**Video 2:** Vertical 9:16 Roblox plastic 3D style. Side-tracking shot: a bandit truck pulls alongside a hauler, trailer doors pop with golden crates flying, escort NPC truck swerves in blocking the bandit, siren flashes, blizzard wall crashes onto the road. Fast cuts, dynamic camera shake.

## 5. Voiceover Script (15s)

Voice: gravelly trucker with a grin. Music: rock-country driving beat.

- 0:00-0:03 `[SFX: radio static, diesel roar]` "Breaker one-nine! The road is calling."
- 0:03-0:07 `[SFX: air horn, tire squeal]` "Haul big loads. Form a convoy. Watch out for bandits!"
- 0:07-0:11 `[SFX: wind howl, bass hit]` "Interstate Rush starts soon. Beat the blizzard!"
- 0:11-0:15 `[SFX: horn blast]` "The Long Haul. Play free on Roblox!"
