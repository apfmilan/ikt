--!strict
-- Balances, oxygen/depth bar, Dive/Surface/Dock buttons.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Remotes = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))
local GameConfig = require(ReplicatedStorage.Shared:WaitForChild("GameConfig"))

local HUDController = {}

local function new(class: string, props: { [string]: any }, parent: Instance?): any
	local inst = Instance.new(class)
	for k, v in props do (inst :: any)[k] = v end
	inst.Parent = parent
	return inst
end

local started = false
local connections: { RBXScriptConnection } = {}

function HUDController.start()
	if started then return end
	started = true

	local playerGui = Players.LocalPlayer:WaitForChild("PlayerGui")
	local gui = new("ScreenGui", { Name = "HUD", ResetOnSpawn = false, IgnoreGuiInset = false }, playerGui)

	-- Balances (top right)
	local balFrame = new("Frame", {
		Size = UDim2.fromScale(0.2, 0.14), Position = UDim2.fromScale(0.79, 0.02),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), BackgroundTransparency = 0.25,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.1, 0) }, balFrame)
	new("UIListLayout", { FillDirection = Enum.FillDirection.Vertical, SortOrder = Enum.SortOrder.LayoutOrder }, balFrame)
	local function balLabel(order: number, color: Color3, text: string): TextLabel
		return new("TextLabel", {
			LayoutOrder = order, Size = UDim2.fromScale(1, 0.33), BackgroundTransparency = 1,
			TextColor3 = color, TextScaled = true, Font = Enum.Font.GothamBold,
			TextXAlignment = Enum.TextXAlignment.Right, Text = text,
		}, balFrame)
	end
	local softLabel = balLabel(1, Color3.fromRGB(255, 220, 120), "Soft: 0")
	local gemLabel = balLabel(2, Color3.fromRGB(120, 220, 255), "Gems: 0")
	local specLabel = balLabel(3, Color3.fromRGB(220, 150, 255), "Special: 0")

	-- Oxygen + depth bar (bottom center)
	local barBack = new("Frame", {
		Size = UDim2.fromScale(0.34, 0.035), Position = UDim2.fromScale(0.33, 0.9),
		BackgroundColor3 = Color3.fromRGB(10, 10, 20), BorderSizePixel = 0,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.5, 0) }, barBack)
	local barFill = new("Frame", {
		Size = UDim2.fromScale(1, 1), BackgroundColor3 = Color3.fromRGB(70, 200, 255), BorderSizePixel = 0,
	}, barBack)
	new("UICorner", { CornerRadius = UDim.new(0.5, 0) }, barFill)
	local barText = new("TextLabel", {
		Size = UDim2.fromScale(1, 1), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.GothamMedium, TextColor3 = Color3.new(1, 1, 1), ZIndex = 2,
		Text = "Docked | Depth 0m | O2 100%",
	}, barBack)

	-- Buttons
	local btnRow = new("Frame", {
		Size = UDim2.fromScale(0.34, 0.06), Position = UDim2.fromScale(0.33, 0.83), BackgroundTransparency = 1,
	}, gui)
	new("UIListLayout", {
		FillDirection = Enum.FillDirection.Horizontal, Padding = UDim.new(0.02, 0),
		HorizontalAlignment = Enum.HorizontalAlignment.Center,
	}, btnRow)
	local function button(text: string, color: Color3, onClick: () -> ()): TextButton
		local b = new("TextButton", {
			Size = UDim2.fromScale(0.31, 1), BackgroundColor3 = color, TextScaled = true,
			Font = Enum.Font.GothamBold, TextColor3 = Color3.new(1, 1, 1), Text = text, AutoButtonColor = true,
		}, btnRow)
		new("UICorner", { CornerRadius = UDim.new(0.2, 0) }, b)
		table.insert(connections, b.Activated:Connect(onClick))
		return b
	end
	local diveReq = Remotes.event("DiveRequest")
	local dockReq = Remotes.event("DockRequest")
	button("Dive", Color3.fromRGB(30, 110, 190), function() diveReq:FireServer("Dive") end)
	button("Surface", Color3.fromRGB(40, 150, 110), function() diveReq:FireServer("Surface") end)
	button("Dock", Color3.fromRGB(120, 90, 50), function() dockReq:FireServer() end)

	local maxO2 = GameConfig.Dive.MaxOxygen

	table.insert(connections, Remotes.event("BalanceUpdate").OnClientEvent:Connect(function(b: any)
		if type(b) ~= "table" then return end
		softLabel.Text = `Soft: {math.floor(tonumber(b.soft) or 0)}`
		gemLabel.Text = `Gems: {math.floor(tonumber(b.gems) or 0)}`
		specLabel.Text = `Special: {math.floor(tonumber(b.special) or 0)}`
	end))

	table.insert(connections, Remotes.event("DiveUpdate").OnClientEvent:Connect(function(d: any)
		if type(d) ~= "table" then return end
		local oxygen = tonumber(d.oxygen) or 0
		local frac = math.clamp(oxygen / maxO2, 0, 1)
		barFill.Size = UDim2.fromScale(frac, 1)
		barFill.BackgroundColor3 = if frac < 0.25 then Color3.fromRGB(230, 60, 60) else Color3.fromRGB(70, 200, 255)
		barText.Text = `{tostring(d.state)} | Depth {math.floor(tonumber(d.depth) or 0)}m | O2 {math.floor(frac * 100)}%`
	end))

	script.Destroying:Once(function()
		for _, c in connections do c:Disconnect() end
		table.clear(connections)
		gui:Destroy()
	end)
end

return HUDController
