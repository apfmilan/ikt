--!strict
-- Defense shop panel. Server validates cost/tier; client only requests.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local UserInputService = game:GetService("UserInputService")

local Remotes = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))
local GameConfig = require(ReplicatedStorage.Shared:WaitForChild("GameConfig"))

local DefenseController = {}

local function new(class: string, props: { [string]: any }, parent: Instance?): any
	local inst = Instance.new(class)
	for k, v in props do (inst :: any)[k] = v end
	inst.Parent = parent
	return inst
end

local started = false
local connections: { RBXScriptConnection } = {}

function DefenseController.start()
	if started then return end
	started = true

	local gui = new("ScreenGui", { Name = "Defense", ResetOnSpawn = false }, Players.LocalPlayer:WaitForChild("PlayerGui"))

	local toggle = new("TextButton", {
		Size = UDim2.fromScale(0.1, 0.05), Position = UDim2.fromScale(0.89, 0.18),
		BackgroundColor3 = Color3.fromRGB(60, 80, 120), TextScaled = true, Font = Enum.Font.GothamBold,
		TextColor3 = Color3.new(1, 1, 1), Text = "Defenses [B]",
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.2, 0) }, toggle)

	local panel = new("Frame", {
		Size = UDim2.fromScale(0.26, 0.5), Position = UDim2.fromScale(0.37, 0.25),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), Visible = false,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.04, 0) }, panel)
	new("UIListLayout", { Padding = UDim.new(0.01, 0), SortOrder = Enum.SortOrder.LayoutOrder }, panel)
	new("TextLabel", {
		LayoutOrder = 0, Size = UDim2.fromScale(1, 0.1), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.GothamBold, TextColor3 = Color3.new(1, 1, 1), Text = "Defense Shop",
	}, panel)
	new("TextLabel", {
		LayoutOrder = 1, Size = UDim2.fromScale(1, 0.07), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.GothamMedium, TextColor3 = Color3.fromRGB(255, 220, 120), Text = "Soft currency only",
	}, panel)

	local place = Remotes.event("PlaceDefense")
	local names: { string } = {}
	for kind in GameConfig.DefenseDefs do table.insert(names, kind) end
	table.sort(names)
	for i, kind in names do
		local def = GameConfig.DefenseDefs[kind]
		local b = new("TextButton", {
			LayoutOrder = 10 + i, Size = UDim2.fromScale(0.94, 0.12), BackgroundColor3 = Color3.fromRGB(35, 70, 100),
			TextScaled = true, Font = Enum.Font.Gotham, TextColor3 = Color3.new(1, 1, 1),
			Text = `{kind} (max tier {def.MaxTier}, from {def.CostPerTier[1]} Soft)`,
		}, panel)
		new("UICorner", { CornerRadius = UDim.new(0.15, 0) }, b)
		table.insert(connections, b.Activated:Connect(function() place:FireServer(kind) end))
	end

	local function setOpen(open: boolean) panel.Visible = open end
	table.insert(connections, toggle.Activated:Connect(function() setOpen(not panel.Visible) end))
	table.insert(connections, UserInputService.InputBegan:Connect(function(input, processed)
		if not processed and input.KeyCode == Enum.KeyCode.B then setOpen(not panel.Visible) end
	end))

	script.Destroying:Once(function()
		for _, c in connections do c:Disconnect() end
		table.clear(connections)
		gui:Destroy()
	end)
end

return DefenseController
