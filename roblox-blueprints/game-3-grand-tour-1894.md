# Game 3: GRAND TOUR 1894 (19th-Century Rail Travel Writer)

Currencies: **Shillings** (soft), **Gems** (premium), **Press Credentials** (special, from scoops and events).
Shared systems: see `00-shared-core.md`.

## 1. Technical & Gameplay Specification

### Core architecture
- `shared/RailGraph` (stations, tracks, travel times), `shared/DispatchDefs` (story types, value, freshness decay).
- `server/Services/TrainService` (scheduled trains, seat assignment), `StationService`, `DispatchService` (write, file, pay), `TelegraphService` (transit + interception), `PressGuardService` (defense), `GrandRaceEvent`.
- `client/Controllers/CarriageController` (interior walk), `NotebookUI` (collect details, compose dispatch), `MapUI`.
- State machine per player: `Station -> Aboard -> Writing -> Filing -> Paid`.

### 24-player capacity
- One continental map with 8 stations on 4 rail lines. Trains are server-scheduled; each carriage seats 6 players, 4 trains active at once (24 seats).
- Only the train a player is on streams its interior; stations stream by distance. Train position is a 1D track value replicated at 2 Hz, clients interpolate.
- Telegraph messages run through a lightweight in-memory queue; no per-frame replication.

### Competitive: Telegraph Interception
- A filed Dispatch travels to the publisher for 60-90s (Exposed) and then pays out (Secured).
- A rival at a telegraph office can intercept: hold a timed Morse-tap minigame. Success claims up to 15% of that Dispatch's payout as "Scoop Credit" (never the whole story). 10-min cooldown per victim, 50% refund.
- Players not at a telegraph office cannot be intercepted; offline players' dispatches are always paid in full.

### Base defense (Shillings only)
| Structure | Effect |
|---|---|
| Cipher Code | Interception minigame gets longer |
| Sealed Pouch | Dispatch skips telegraph, delivered by next train (slower, safe) |
| Press Guard | Detects and alerts on interception |
| Private Compartment Lock | Keeps rivals out of your seat/notes area |
| Courier Pigeon | Splits dispatch into two paths, halving max steal |

### Community event: The Great Race (every 10-15 min)
60s warning. All trains race to the Grand Terminus; en route, pickups (landmark sketches, rare quotes) appear along carriages and platforms. Players collect and file a "Grand Dispatch" for score. Top 3 get Press Credentials and a gilded notebook skin; participants over threshold get Shillings and Gems.

### Interactive tutorial
1. Board a train. 2. Collect three details in the notebook. 3. File a first Dispatch. 4. Watch it travel and get paid. 5. Try a safe interception drill on a practice dispatch. 6. Buy a Cipher Code. 7. Currency Crash Course (Shillings / Gems / Press Credentials, optional shop). 8. Reward: 25 Gems + 1 Press Credential.

## 2. Monetization Schema

```lua
--!strict
return {
	GamePasses = {
		{ Key = "VIPTag", Id = 0, Name = "First Class VIP", Description = "Golden ticket VIP tag and a first-class carriage skin.", PriceRobux = 249, Type = "VIP" },
		{ Key = "VictorianWardrobe", Id = 0, Name = "Victorian Wardrobe", Description = "Twelve period outfits: top hats, travel coats and dresses.", PriceRobux = 399, Type = "Cosmetic" },
		{ Key = "AutoNotebook", Id = 0, Name = "Smart Notebook", Description = "Auto-organizes your collected details by story. Quality of life only.", PriceRobux = 199, Type = "QoL" },
		{ Key = "FastMap", Id = 0, Name = "Route Planner", Description = "Pin favorite routes on the map. Does not change travel times.", PriceRobux = 149, Type = "QoL" },
	},
	DevProducts = {
		{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
		{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
		{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
		{ Key = "RaceKey", Id = 0, Name = "Grand Race Key", Description = "Unlocks the bonus cosmetic reward track for one Grand Race. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
		{ Key = "DisplayCase", Id = 0, Name = "Souvenir Display Case", Description = "A glass case in your carriage for souvenirs and framed front pages.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
		{ Key = "BlueprintSlot", Id = 0, Name = "Journal Slot", Description = "One more slot to store finished dispatches as keepsakes.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
	},
}
```

## 3. Image Prompts

**Icon (512x512, `--ar 1:1`):** Stylized Roblox 3D character in a Victorian travel coat and top hat leaning out of a steam locomotive window with a glowing notebook and fountain pen, golden sunset over mountains, steam clouds, warm brass and teal palette, bold outlines, strong center focus, no text.

**Thumbnail (1920x1080, `--ar 16:9`):** Wide cinematic Roblox-style scene of several 1894 steam trains racing across a grand viaduct through snowy mountains and a glowing city skyline, writers hanging from windows holding telegrams, flying paper dispatches, sunset gold and deep blue palette, space on the left for the title "GRAND TOUR 1894", adventurous and energetic mood.

## 4. Video Prompts (9:16)

**Video 1:** Vertical 9:16 Roblox 3D stylized look. Interior of a gleaming steam-train carriage, camera glides past a writer scribbling in a notebook as the landscape rushes by outside, then whip-pans out the window to the train crossing a huge bridge at sunset, steam swirling. Golden warm light, smooth tracking shot, ends on a glowing telegram stamping "FILED".

**Video 2:** Vertical 9:16 Roblox plastic 3D style. A telegraph office: player taps Morse on a brass key as a glowing dispatch scroll zips along a wire, rival hand reaches in and snatches a glowing corner, alarm bell rings. Fast cut to trains racing side by side to the terminus, confetti, dynamic camera shake.

## 5. Voiceover Script (15s)

Voice: charming, fast narrator. Music: upbeat ragtime meets modern beat.

- 0:00-0:03 `[SFX: train whistle, steam hiss]` "It's 1894! Board the grandest rails on earth!"
- 0:03-0:07 `[SFX: typewriter, telegraph tap]` "Write the story. File the scoop. But beware, rivals are listening!"
- 0:07-0:11 `[SFX: bell, crowd cheer]` "Join the Great Race and top the server!"
- 0:11-0:15 `[SFX: whistle blast]` "Grand Tour 1894. Play free on Roblox!"
