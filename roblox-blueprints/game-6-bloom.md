# Game 6: BLOOM (Asynchronous Moon Terraforming & Genetics)

Currencies: **Spores** (soft), **Gems** (premium), **Gene Tokens** (special, from rare mutations and events).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- `shared/GeneDefs` (trait genes, dominance, mutation table), `shared/PlantDefs` (growth time, yield), `shared/GeneticsMath` (pure, unit-testable breeding).
- `server/Services/DomeService` (24 domes), `GrowthService` (timestamp-based offline growth), `BreedingService`, `HarvestService`, `SporeRaidService`, `DefenseService`, `EclipseBloomEvent`.
- `client/Controllers/DomeController`, `GeneLabUI` (Punnett-style trait picker), `MoonMapUI`.
- Asynchronous core: `GrowthService` stores `plantedAt` and `readyAt` UTC timestamps; progress is computed on join, no loop required.

### 24-player capacity
- 24 domes arranged in a ring on a lunar crater, each with 12 planter beds. A central Terraforming Core shows collective moon progress.
- Offline-heavy design means active load is low: only online players run visual tickers. Dome visuals stream by distance.
- Shared moon progress uses a MemoryStore counter and an OrderedDataStore for cross-server totals.

### Competitive: Spore Raids (opt-in, safe)
- Mature crops in **Open Beds** can be harvested early by raiders running drones through the dome (5s skill-path minigame). Max 15% of harvest-ready yield; **Sealed Beds** (default after planting) are never raidable.
- Players choose which beds stay Open for the Bloom Market bonus (+10% yield). Offline domes are protected unless the owner explicitly opened a bed; 50% refund; 10-min per-victim cooldown.
- Gene Tokens and rare genes can never be stolen.

### Base defense (Spores only)
| Structure | Effect |
|---|---|
| Pollen Shield | Harder drone path minigame |
| Sentry Bees | NPC bees nudge raider drones off path |
| Glass Barrier | Lowers visibility of crops from outside |
| Irrigation Lock | Locks bed's harvest to owner for 2 min after ripening |
| Auto-Seal Timer | Re-seals beds after harvest window |

### Community event: Eclipse Bloom (every 10-15 min)
60s warning. During a 3-minute eclipse, special Lunar Seeds can be planted and instantly fast-grown in the central Grove. Players cooperate by watering and pollinating the shared Grove; individual contribution is tracked. Top 3 get Gene Tokens and a glow-bloom skin; participants over threshold get Spores and small Gems.

### Interactive tutorial
1. Plant a seed. 2. Fast-forward demo growth. 3. Harvest. 4. Breed two plants in the Gene Lab and see traits. 5. Open a bed to the Bloom Market and see a safe raid drill. 6. Place a Pollen Shield. 7. Currency Crash Course (Spores / Gems / Gene Tokens, optional shop). 8. Reward: 25 Gems + 1 Gene Token.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "Moon Botanist VIP", Description = "Glowing VIP tag and a luminous crater path to your dome.", PriceRobux = 249, Type = "VIP" },
		{ Key = "DomeSkins", Id = 0, Name = "Dome Skin Pack", Description = "Cosmetic dome glass, trims and ambient lighting styles.", PriceRobux = 399, Type = "Cosmetic" },
		{ Key = "AutoHarvestUI", Id = 0, Name = "Quick Harvest Menu", Description = "One-click harvest-all button. Does not speed up growth.", PriceRobux = 199, Type = "QoL" },
		{ Key = "GeneNotes", Id = 0, Name = "Gene Notebook", Description = "Save notes and favorites on your gene lines.", PriceRobux = 99, Type = "QoL" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "EclipseKey", Id = 0, Name = "Eclipse Bloom Key", Description = "Unlocks the bonus cosmetic reward track for one Eclipse. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Specimen Display Case", Description = "A glass case in your dome for your favorite rare blooms.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "Gene Line Slot", Description = "Store one more bred gene line in your library. Does not grant genes.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts

**Icon (512x512, `--ar 1:1`):** Stylized Roblox 3D glass dome on a gray moon surface full of glowing multicolor alien flowers, Earth rising in a dark starry sky behind, tiny astronaut gardener with a watering can, bold outlines, teal, magenta and gold bioluminescent palette, strong center focus, no text.

**Thumbnail (1920x1080, `--ar 16:9`):** Wide cinematic Roblox-style lunar crater with a ring of glowing glass domes, giant luminous blooms and vines climbing toward the sky, small drones flying between domes, eclipse in the sky with a golden ring, astronauts cheering around a central glowing grove, space on the left for the title "BLOOM", wondrous and cozy-sci-fi mood.

## 4. Video Prompts (9:16)

**Video 1:** Vertical 9:16 Roblox 3D stylized look. Macro shot of a seed in moon soil, time-lapse: sprout, stem, glowing petals unfurl, camera pulls back through glass dome revealing a field of luminous flowers under a starry sky. Soft bloom lighting, smooth dolly-out, magical sparkles.

**Video 2:** Vertical 9:16 Roblox plastic 3D style. Gene lab: two flowers combine in a glowing helix animation producing a new rainbow bloom, then a drone zips through a rival dome along a glowing path, bees swarm out and bump it, eclipse darkens the sky and a golden ring lights up. Fast cuts, vibrant colors.

## 5. Voiceover Script (15s)

Voice: warm, curious female. Music: dreamy synth into uplifting beat.

- 0:00-0:03 `[SFX: soft chime, whoosh]` "Plant a seed on the moon."
- 0:03-0:07 `[SFX: sparkle, growth pop]` "Breed rare genes. Watch them bloom, even while you're away!"
- 0:07-0:11 `[SFX: bee buzz, drone whirr]` "Defend your garden from sneaky drones."
- 0:11-0:15 `[SFX: bell chime]` "Bloom. Play free on Roblox!"
