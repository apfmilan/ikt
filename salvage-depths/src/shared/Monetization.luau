--!strict
local Types = require(script.Parent.Types)

type Def = Types.MonetizationDef

local GamePasses: { Def } = {
	{ Key = "VIPTag", Id = 0, Name = "Deep Diver VIP", Description = "Gold VIP tag in chat, VIP dock banner and a unique sonar ping color.", PriceRobux = 249, Type = "VIP" },
	{ Key = "NeonHulls", Id = 0, Name = "Neon Hull Pack", Description = "Five glowing hull paint jobs and a bioluminescent trail.", PriceRobux = 399, Type = "Cosmetic" },
	{ Key = "AutoSort", Id = 0, Name = "Auto-Sort Cargo", Description = "Automatically sorts your hold by value and rarity. Quality of life only.", PriceRobux = 199, Type = "QoL" },
	{ Key = "ExtraLoadouts", Id = 0, Name = "Extra Loadout Slots", Description = "Save 5 more cosmetic and gear loadouts. No stat advantage.", PriceRobux = 149, Type = "QoL" },
}

local DevProducts: { Def } = {
	{ Key = "Gems100", Id = 0, Name = "Gem Pouch (100)", Description = "100 Gems for cosmetics, slots and display cases.", PriceRobux = 49, Type = "Currency", Grant = { Gems = 100 } },
	{ Key = "Gems550", Id = 0, Name = "Gem Chest (550)", Description = "550 Gems for cosmetics, slots and display cases.", PriceRobux = 249, Type = "Currency", Grant = { Gems = 550 } },
	{ Key = "Gems1200", Id = 0, Name = "Gem Vault (1200)", Description = "1200 Gems for cosmetics, slots and display cases.", PriceRobux = 499, Type = "Currency", Grant = { Gems = 1200 } },
	{ Key = "SurgeKey", Id = 0, Name = "Trench Surge Key", Description = "Unlocks the bonus cosmetic reward track for one Surge. No gameplay advantage.", PriceRobux = 49, Type = "EventKey", Grant = { EventKeys = 1 } },
	{ Key = "DisplayCase", Id = 0, Name = "Relic Display Case", Description = "A glass case in your dock to show off found Artifacts.", PriceRobux = 99, Type = "DisplayCase", Grant = { DisplayCases = 1 } },
	{ Key = "BlueprintSlot", Id = 0, Name = "Sub Blueprint Slot", Description = "One more slot to store sub designs you have already unlocked.", PriceRobux = 149, Type = "BlueprintSlot", Grant = { BlueprintSlots = 1 } },
}

-- Id 0 = placeholder (not yet created in Creator Hub); skipped in lookups.
local byProductId: { [number]: Def } = {}
local byPassId: { [number]: Def } = {}
for _, d in DevProducts do
	if d.Id ~= 0 then byProductId[d.Id] = d end
end
for _, d in GamePasses do
	if d.Id ~= 0 then byPassId[d.Id] = d end
end

return {
	GamePasses = GamePasses,
	DevProducts = DevProducts,
	byProductId = byProductId,
	byPassId = byPassId,
}
