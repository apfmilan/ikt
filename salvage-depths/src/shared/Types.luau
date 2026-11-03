--!strict
export type ProfileData = {
	Version: number,
	Soft: number, Gems: number, Special: number,
	Receipts: { [string]: boolean },
	GamePasses: { [string]: boolean },
	TutorialStep: number,
	Defenses: { [string]: number },        -- kind -> tier owned
	SecuredCargo: number,
	LastRaidedAt: number,
	ShieldUntil: number,                    -- unix seconds
	RaidedBy: { [string]: number },         -- attackerUserId -> last raid unix time (cooldown)
	EventKeys: number, DisplayCases: number, BlueprintSlots: number,
	Peaceful: boolean,
}
export type Balance = { soft: number, gems: number, special: number }
export type ItemType = "Cosmetic" | "VIP" | "QoL" | "Currency" | "EventKey" | "DisplayCase" | "BlueprintSlot"
export type MonetizationDef = {
	Key: string, Id: number, Name: string, Description: string,
	PriceRobux: number, Type: ItemType, Grant: { [string]: any }?,
}
return {}
