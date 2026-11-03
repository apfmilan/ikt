# Game 8: PRECINCT (Player-Generated Mystery Detective)

Currencies: **Badge Pay** (soft), **Gems** (premium), **Case Files** (special, from top-rated cases and events).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- `shared/CaseSchema` (typed clue graph: Scene, Clue, Suspect, Alibi, Solution), `shared/TemplateDefs` (curated rooms, props, suspect pools). Authors assemble cases from templates; no free-form scripting.
- `server/Services/CaseAuthorService` (validate, publish), `CaseBrowserService` (rating, tags), `InvestigationService` (server holds solutions, checks answers), `ModerationService` (text filtering and reports), `EvidenceRaidService`, `PrecinctSecurityService`, `ManhuntEvent`.
- `client/Controllers/CaseBoardUI` (string-and-pin board), `AuthorUI` (graph editor), `BrowserUI`.
- State machine per investigation: `Briefing -> Investigating -> Accusing -> Verdict`.

### UGC safety (required)
- All author text passes `TextService:FilterStringAsync` before storage and again for display to each viewer. Case publish is rate-limited and requires playtest-solve by the author. Players can report cases; 3 unique reports auto-hide pending review. Solutions are never sent to clients.

### 24-player capacity
- One shared Precinct hub with 24 desks. Each player's active case runs in an instanced scene on the server (reused template models, cap 25 live props). Cases load from DataStore by id; popular cases cached in MemoryStore.
- Hub shows leaderboards and the live Case Browser; only active scenes stream to their solvers.

### Competitive: Evidence Wiretap
- Clues you've found sit on your **Case Board** (Exposed) until you file the report (Secured, counts toward score).
- A rival can wiretap an unlocked board: a short decode minigame lets them copy up to 15% of your found clues for the same case (they still must solve it themselves; you lose nothing but exclusivity bonus). 10-min cooldown per victim, you receive a 50% "exclusivity refund".
- Solutions and author-only data are never exposed. Offline boards are locked.

### Base defense (Badge Pay only)
| Structure | Effect |
|---|---|
| Evidence Locker | Secures clues faster |
| Encrypted Case File | Makes decode minigame longer |
| Security Camera | Marks the wiretapper and logs their name |
| Desk Sergeant (NPC) | Blocks one wiretap attempt per case |
| Frosted Glass | Hides your board layout from nearby players |

### Community event: Citywide Manhunt (every 10-15 min)
60s warning. A special featured case (pinned by curators from top-rated submissions) opens for everyone at once. First to correctly accuse wins; ranking by time and accuracy. Top 3 get Case Files plus a gold badge skin; correct solvers get Badge Pay and small Gems; the author earns a bonus.

### Interactive tutorial
1. Read a briefing and inspect a scene. 2. Pin three clues. 3. Accuse a suspect. 4. Try authoring a tiny case with a template. 5. See a wiretap drill and buy an Evidence Locker. 6. Currency Crash Course (Badge Pay / Gems / Case Files, optional shop). 7. Reward: 25 Gems + 1 Case File.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "Chief Inspector VIP", Description = "Gold badge VIP tag, VIP desk nameplate and trench-coat chat color.", PriceRobux = 249, Type = "VIP" },
		{ Key = "NoirPack", Id = 0, Name = "Noir Style Pack", Description = "Cosmetic detective outfits, desk props and case-board themes.", PriceRobux = 399, Type = "Cosmetic" },
		{ Key = "AuthorStudio", Id = 0, Name = "Author Studio Pro", Description = "More template slots, drafts and an undo history in the case editor. Cases are not boosted in ranking.", PriceRobux = 249, Type = "QoL" },
		{ Key = "CaseBookmarks", Id = 0, Name = "Case Bookmarks", Description = "Bookmark and tag unlimited cases in the browser.", PriceRobux = 99, Type = "QoL" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "ManhuntKey", Id = 0, Name = "Manhunt Key", Description = "Unlocks the bonus cosmetic reward track for one Manhunt. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Evidence Display Case", Description = "A glass case on your desk for solved-case trophies.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "Case Template Slot", Description = "Store one more case template draft. Does not boost visibility or rating.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts

**Icon (512x512, `--ar 1:1`):** Stylized Roblox 3D detective in a trench coat and fedora holding a glowing magnifying glass over a corkboard of connected red strings and photos, moody neon-noir city window behind, bold outlines, amber, navy and red palette, strong center focus, no text.

**Thumbnail (1920x1080, `--ar 16:9`):** Wide cinematic Roblox-style police precinct at night, rows of detective desks with glowing case boards, a giant corkboard in the center with red string connecting suspects, rain-streaked windows with a neon city, players pointing at clues, space on the left for the title "PRECINCT", clever mystery mood suitable for all ages.

## 4. Video Prompts (9:16)

**Video 1:** Vertical 9:16 Roblox 3D stylized look. Slow dolly into a dim detective office, a magnifying glass sweeps across a crime scene and a clue glows, red string zips across a corkboard connecting photos, a suspect silhouette lights up, text-free reveal. Moody neon lighting, rain on window, quick snap zoom.

**Video 2:** Vertical 9:16 Roblox plastic 3D style. Busy precinct hub: dozens of detectives sprint toward the central board as a siren spins, a featured case opens with a big stamp slam, one player slams the "ACCUSE" button and confetti bursts. Handheld energy, fast cuts, vibrant colors.

## 5. Voiceover Script (15s)

Voice: dramatic noir narrator turning punchy. Music: jazzy sax into modern beat.

- 0:00-0:03 `[SFX: rain, sax sting]` "Every case starts with a clue."
- 0:03-0:07 `[SFX: stamp, string zip]` "Solve mysteries built by players. Or build your own!"
- 0:07-0:11 `[SFX: siren, bass drop]` "When the Manhunt starts, the whole city races for the verdict."
- 0:11-0:15 `[SFX: gavel]` "Precinct. Play free on Roblox!"
