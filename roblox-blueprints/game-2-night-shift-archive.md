# Game 2: NIGHT SHIFT AT THE ARCHIVE (Library Horror / Puzzle)

Currencies: **Ink** (soft), **Gems** (premium), **Relic Pages** (special, from rare puzzles and events).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- `shared/PuzzleDefs` (cipher, shelf-order, light-mirror, rune-lock), `shared/WraithConfig`.
- `server/Services/PuzzleService` (server validates all solutions), `WraithService` (patrol AI with waypoint graph), `WingService` (24 reading rooms), `TomeService` (Tome Fragments), `InkThiefService` (raids), `WardService` (defense), `MidnightBellEvent`.
- `client/Controllers/FlashlightController`, `PuzzleUI`, `AmbienceController`.
- State machine: `Quiet -> Stirring -> Haunted -> Dawn`.

### 24-player capacity
- 24 private Reading Rooms around one central Great Hall. Each room is a pre-built model slot with its own puzzle seeds; streaming loads rooms by distance.
- Only 1 Shelf Wraith per 8 players (max 3), running on a shared waypoint graph at 10 Hz server time; clients get position snapshots and interpolate.
- Puzzle state per room is server-side and replicates only to the owner and nearby players.

### Competitive: Ink Thief raids
- Solved puzzles produce **Tome Fragments** stored on your desk (Exposed) until bound into a Tome at the Binding Press (Secured).
- A rival can sneak into your room and take fragments from the desk by completing a 5s "Quiet Lift" timing minigame. Max 15% of Exposed fragments per raid. Moving through the Great Hall draws the Wraith toward both players.
- 10-min per-victim cooldown, 50% refund, offline rooms are locked.

### Base defense (Ink only)
| Structure | Effect |
|---|---|
| Ward Sigil | Raider's Quiet Lift minigame gets harder |
| Lock-Seal | Door needs a 3-digit rune code the owner sets |
| Hall Monitor (NPC) | Alerts owner and slows the raider briefly |
| Warding Lantern | Reveals raiders in the room outline |
| Reinforced Shelf | Raises Secured Tome capacity |

### Community event: Midnight Bell (every 10-15 min)
60s warning; the Bell tolls, lights go out, all Wraiths enter hunt mode for 3 minutes. Players race to light candles and solve the shared Great Hall cipher (fragments revealed per solved candle). Top 3 solvers get Relic Pages + exclusive lantern skin; all who lit 3+ candles get Ink and small Gems.

### Interactive tutorial
1. Pick up flashlight and read a clue. 2. Solve a shelf-order puzzle. 3. Hide from a scripted Wraith. 4. Bind fragments in the Press (Exposed to Secured). 5. Place a Ward Sigil. 6. Currency Crash Course (Ink / Gems / Relic Pages tracks and optional shop). 7. Reward: 25 Gems + 1 Relic Page.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "Head Archivist VIP", Description = "Violet VIP tag, special reading-lamp glow and a VIP chat badge.", PriceRobux = 249, Type = "VIP" },
		{ Key = "GothicRoom", Id = 0, Name = "Gothic Room Decor", Description = "Cosmetic wallpaper, rugs and portraits for your Reading Room.", PriceRobux = 349, Type = "Cosmetic" },
		{ Key = "ClueJournal", Id = 0, Name = "Clue Journal Pro", Description = "A neater journal with sorting tabs and highlights. Does not reveal answers.", PriceRobux = 199, Type = "QoL" },
		{ Key = "FlashlightSkins", Id = 0, Name = "Flashlight Skin Set", Description = "Six cosmetic flashlight styles. Light radius unchanged.", PriceRobux = 149, Type = "Cosmetic" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "BellKey", Id = 0, Name = "Midnight Bell Key", Description = "Unlocks the bonus cosmetic reward track for one Bell. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Tome Display Case", Description = "A glass case in your room to display your rarest bound Tomes.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "Room Layout Slot", Description = "Save one more room decor layout. Cosmetic only.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts

**Icon (512x512, `--ar 1:1`):** Stylized Roblox 3D character holding a glowing flashlight in a huge dark gothic library, towering bookshelves, a translucent ghostly librarian wraith with glowing eyes looming behind, deep purple and warm amber lighting, bold outlines, strong central focus, no text, readable at small size.

**Thumbnail (1920x1080, `--ar 16:9`):** Wide cinematic Roblox-style scene of a gigantic moonlit archive hall, several players with flashlights solving glowing rune puzzles, a tall spectral Shelf Wraith emerging from the shelves, floating pages and candles, volumetric fog, purple, teal and gold palette, space on the right for the title "NIGHT SHIFT AT THE ARCHIVE", dramatic horror-lite mood suitable for all ages.

## 4. Video Prompts (9:16)

**Video 1:** Vertical 9:16 Roblox 3D stylized look. First-person flashlight sweeping a dark library aisle, books floating, a rune puzzle lights up on a wall as the player rotates it, doors slam shut, a pale wraith silhouette glides past at the end of the aisle. Slow push forward, flickering light, quick jump-scare cut to the puzzle unlocking with golden sparks.

**Video 2:** Vertical 9:16 Roblox plastic 3D style. Tracking shot of a player sneaking into a rival's reading room, hand lifting glowing page fragments from a desk, a ward sigil flares purple, alarm lantern flashes, the player sprints out as a giant bell tolls and all the lights go out. Handheld camera, fast cuts, high contrast.

## 5. Voiceover Script (15s)

Voice: whispery then punchy, female. Music: eerie piano into beat drop.

- 0:00-0:03 `[SFX: bell toll, whisper]` "The archive never sleeps. Neither do its secrets."
- 0:03-0:07 `[SFX: page rustle, rune chime]` "Solve the puzzles. Bind the tomes. But watch your desk!"
- 0:07-0:11 `[SFX: bass drop, ghost wail]` "When the Midnight Bell tolls, the whole server runs!"
- 0:11-0:15 `[SFX: door slam]` "Night Shift at the Archive. Play free on Roblox."
