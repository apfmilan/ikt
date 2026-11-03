--!strict
-- Exposed vs Secured cargo display + salvage prompts on "Wreck"-tagged parts.
local CollectionService = game:GetService("CollectionService")
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Remotes = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))

local CargoController = {}

local function new(class: string, props: { [string]: any }, parent: Instance?): any
	local inst = Instance.new(class)
	for k, v in props do (inst :: any)[k] = v end
	inst.Parent = parent
	return inst
end

local started = false
local connections: { RBXScriptConnection } = {}
local prompts: { [Instance]: ProximityPrompt } = {}

function CargoController.start()
	if started then return end
	started = true

	local playerGui = Players.LocalPlayer:WaitForChild("PlayerGui")
	local gui = new("ScreenGui", { Name = "Cargo", ResetOnSpawn = false }, playerGui)
	local frame = new("Frame", {
		Size = UDim2.fromScale(0.2, 0.12), Position = UDim2.fromScale(0.01, 0.02),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), BackgroundTransparency = 0.25,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.1, 0) }, frame)
	new("UIListLayout", { SortOrder = Enum.SortOrder.LayoutOrder }, frame)
	local function line(order: number, color: Color3): TextLabel
		return new("TextLabel", {
			LayoutOrder = order, Size = UDim2.fromScale(1, 0.33), BackgroundTransparency = 1,
			TextScaled = true, Font = Enum.Font.GothamBold, TextColor3 = color,
			TextXAlignment = Enum.TextXAlignment.Left, Text = "",
		}, frame)
	end
	local exposedLabel = line(1, Color3.fromRGB(255, 120, 100))
	local securedLabel = line(2, Color3.fromRGB(110, 230, 140))
	local capLabel = line(3, Color3.fromRGB(200, 200, 200))
	exposedLabel.Text = "Exposed: 0 (raidable)"
	securedLabel.Text = "Secured: 0 (safe)"
	capLabel.Text = "Capacity: 0"

	table.insert(connections, Remotes.event("CargoUpdate").OnClientEvent:Connect(function(c: any)
		if type(c) ~= "table" then return end
		exposedLabel.Text = `Exposed: {math.floor(tonumber(c.exposed) or 0)} (raidable)`
		securedLabel.Text = `Secured: {math.floor(tonumber(c.secured) or 0)} (safe)`
		capLabel.Text = `Capacity: {math.floor(tonumber(c.capacity) or 0)}`
	end))

	local salvageReq = Remotes.event("SalvageRequest")

	local function addWreck(inst: Instance)
		if not inst:IsA("BasePart") or prompts[inst] then return end
		local prompt = new("ProximityPrompt", {
			ActionText = "Salvage", ObjectText = "Wreck", HoldDuration = 1.5,
			MaxActivationDistance = 12, RequiresLineOfSight = false,
		}, inst) :: ProximityPrompt
		prompts[inst] = prompt
		prompt.Triggered:Connect(function()
			local id = inst:GetAttribute("WreckId")
			if type(id) == "string" or type(id) == "number" then
				salvageReq:FireServer(tostring(id))
			end
		end)
	end
	local function removeWreck(inst: Instance)
		local p = prompts[inst]
		if p then
			prompts[inst] = nil
			p:Destroy() -- also drops the Triggered connection
		end
	end

	for _, inst in CollectionService:GetTagged("Wreck") do addWreck(inst) end
	table.insert(connections, CollectionService:GetInstanceAddedSignal("Wreck"):Connect(addWreck))
	table.insert(connections, CollectionService:GetInstanceRemovedSignal("Wreck"):Connect(removeWreck))

	script.Destroying:Once(function()
		for _, c in connections do c:Disconnect() end
		table.clear(connections)
		for inst in prompts do removeWreck(inst) end
		gui:Destroy()
	end)
end

return CargoController
