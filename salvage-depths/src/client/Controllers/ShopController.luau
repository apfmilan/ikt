--!strict
-- Optional shop. Never required to progress. Catalog comes from Shared.Monetization (lazy, pcall).
local MarketplaceService = game:GetService("MarketplaceService")
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local UserInputService = game:GetService("UserInputService")

local ShopController = {}

local function new(class: string, props: { [string]: any }, parent: Instance?): any
	local inst = Instance.new(class)
	for k, v in props do (inst :: any)[k] = v end
	inst.Parent = parent
	return inst
end

local started = false
local connections: { RBXScriptConnection } = {}

-- Catalog shape: { GamePasses = { def }, DevProducts = { def } }. Tag each def by its list.
local function collectItems(catalog: any): { any }
	local items: { any } = {}
	for _, def in (catalog.GamePasses or {}) do
		table.insert(items, { def = def, pass = true })
	end
	for _, def in (catalog.DevProducts or {}) do
		table.insert(items, { def = def, pass = false })
	end
	return items
end

function ShopController.start()
	if started then return end
	started = true

	local player = Players.LocalPlayer
	local gui = new("ScreenGui", { Name = "Shop", ResetOnSpawn = false }, player:WaitForChild("PlayerGui"))

	local toggle = new("TextButton", {
		Size = UDim2.fromScale(0.1, 0.05), Position = UDim2.fromScale(0.89, 0.24),
		BackgroundColor3 = Color3.fromRGB(120, 70, 150), TextScaled = true, Font = Enum.Font.GothamBold,
		TextColor3 = Color3.new(1, 1, 1), Text = "Shop [M]",
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.2, 0) }, toggle)

	local panel = new("Frame", {
		Size = UDim2.fromScale(0.3, 0.55), Position = UDim2.fromScale(0.35, 0.22),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), Visible = false,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.04, 0) }, panel)
	new("TextLabel", {
		Size = UDim2.fromScale(1, 0.09), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.GothamBold, TextColor3 = Color3.new(1, 1, 1), Text = "Optional Shop",
	}, panel)
	new("TextLabel", {
		Size = UDim2.fromScale(1, 0.06), Position = UDim2.fromScale(0, 0.09), BackgroundTransparency = 1,
		TextScaled = true, Font = Enum.Font.Gotham, TextColor3 = Color3.fromRGB(255, 230, 150),
		Text = "Nothing here is required. Gems never buy defenses or raid power.",
	}, panel)
	local list = new("ScrollingFrame", {
		Size = UDim2.fromScale(0.96, 0.8), Position = UDim2.fromScale(0.02, 0.17), BackgroundTransparency = 1,
		CanvasSize = UDim2.new(), AutomaticCanvasSize = Enum.AutomaticSize.Y, ScrollBarThickness = 4, BorderSizePixel = 0,
	}, panel)
	new("UIListLayout", { Padding = UDim.new(0, 4) }, list)
	local status = new("TextLabel", {
		Size = UDim2.new(1, 0, 0, 30), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.Gotham, TextColor3 = Color3.new(1, 1, 1), Text = "Loading...",
	}, list)

	local populated = false
	local function populate()
		if populated then return end
		local ok, catalog = pcall(function()
			local shared = ReplicatedStorage:FindFirstChild("Shared")
			local mod = shared and shared:FindFirstChild("Monetization")
			if not mod or not mod:IsA("ModuleScript") then error("catalog missing") end
			return require(mod)
		end)
		if not ok then
			status.Text = "Shop unavailable right now."
			return
		end
		local items = collectItems(catalog)
		if #items == 0 then
			status.Text = "Nothing for sale yet."
			return
		end
		populated = true
		status:Destroy()
		for _, item in items do
			local def = item.def
			local b = new("TextButton", {
				Size = UDim2.new(1, 0, 0, 36), BackgroundColor3 = Color3.fromRGB(45, 60, 100),
				TextScaled = true, Font = Enum.Font.Gotham, TextColor3 = Color3.new(1, 1, 1),
				Text = `{def.Name} - R${tostring(def.PriceRobux or "?")}`,
			}, list)
			new("UICorner", { CornerRadius = UDim.new(0.15, 0) }, b)
			local id = def.Id :: number
			local pass = item.pass
			b.Activated:Connect(function()
				if pass then
					MarketplaceService:PromptGamePassPurchase(player, id)
				else
					MarketplaceService:PromptProductPurchase(player, id)
				end
			end)
		end
	end

	local function setOpen(open: boolean)
		panel.Visible = open
		if open then populate() end
	end
	table.insert(connections, toggle.Activated:Connect(function() setOpen(not panel.Visible) end))
	table.insert(connections, UserInputService.InputBegan:Connect(function(input, processed)
		if not processed and input.KeyCode == Enum.KeyCode.M then setOpen(not panel.Visible) end
	end))

	script.Destroying:Once(function()
		for _, c in connections do c:Disconnect() end
		table.clear(connections)
		gui:Destroy()
	end)
end

return ShopController
