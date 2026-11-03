--!strict
-- Tutorial step panel + Currency Crash Course (3 earn tracks, optional shop note).
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Remotes = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))

local TutorialController = {}

local function new(class: string, props: { [string]: any }, parent: Instance?): any
	local inst = Instance.new(class)
	for k, v in props do (inst :: any)[k] = v end
	inst.Parent = parent
	return inst
end

local started = false
local connections: { RBXScriptConnection } = {}

function TutorialController.start()
	if started then return end
	started = true

	local gui = new("ScreenGui", { Name = "Tutorial", ResetOnSpawn = false }, Players.LocalPlayer:WaitForChild("PlayerGui"))
	local ack = Remotes.event("TutorialAck")

	-- Step panel
	local stepPanel = new("Frame", {
		Size = UDim2.fromScale(0.34, 0.12), Position = UDim2.fromScale(0.33, 0.1),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), BackgroundTransparency = 0.1, Visible = false,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.1, 0) }, stepPanel)
	local stepText = new("TextLabel", {
		Size = UDim2.fromScale(0.96, 0.9), Position = UDim2.fromScale(0.02, 0.05), BackgroundTransparency = 1,
		TextScaled = true, TextWrapped = true, Font = Enum.Font.GothamMedium, TextColor3 = Color3.new(1, 1, 1), Text = "",
	}, stepPanel)

	-- Currency crash course
	local crash = new("Frame", {
		Size = UDim2.fromScale(0.5, 0.6), Position = UDim2.fromScale(0.25, 0.2),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), Visible = false, ZIndex = 5,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.04, 0) }, crash)
	new("TextLabel", {
		Size = UDim2.fromScale(1, 0.12), BackgroundTransparency = 1, TextScaled = true, ZIndex = 5,
		Font = Enum.Font.GothamBold, TextColor3 = Color3.new(1, 1, 1), Text = "Currency Crash Course",
	}, crash)
	local tracks = {
		{ "Soft", "Earn: salvage wrecks and bring cargo home. Spend: defenses and upgrades.", Color3.fromRGB(255, 220, 120) },
		{ "Gems", "Earn: tutorial, events, daily play. Spend: cosmetics and conveniences.", Color3.fromRGB(120, 220, 255) },
		{ "Special", "Earn: events and rare finds. Spend: display cases, blueprint slots, event keys.", Color3.fromRGB(220, 150, 255) },
	}
	for i, t in tracks do
		local card = new("Frame", {
			Size = UDim2.fromScale(0.3, 0.5), Position = UDim2.fromScale(0.03 + (i - 1) * 0.32, 0.15),
			BackgroundColor3 = Color3.fromRGB(25, 50, 75), ZIndex = 5,
		}, crash)
		new("UICorner", { CornerRadius = UDim.new(0.06, 0) }, card)
		new("TextLabel", {
			Size = UDim2.fromScale(1, 0.2), BackgroundTransparency = 1, TextScaled = true, ZIndex = 5,
			Font = Enum.Font.GothamBold, TextColor3 = t[3], Text = t[1],
		}, card)
		new("TextLabel", {
			Size = UDim2.fromScale(0.92, 0.75), Position = UDim2.fromScale(0.04, 0.22), BackgroundTransparency = 1,
			TextScaled = true, TextWrapped = true, ZIndex = 5, Font = Enum.Font.Gotham,
			TextColor3 = Color3.new(1, 1, 1), TextYAlignment = Enum.TextYAlignment.Top, Text = t[2],
		}, card)
	end
	local note = new("TextLabel", {
		Size = UDim2.fromScale(0.94, 0.1), Position = UDim2.fromScale(0.03, 0.68), BackgroundTransparency = 1,
		TextScaled = true, TextWrapped = true, ZIndex = 5, Font = Enum.Font.GothamMedium,
		TextColor3 = Color3.fromRGB(255, 230, 150),
		Text = "Optional shop: Gems never buy defenses or raid power. You can skip it entirely.",
	}, crash)
	local cont = new("TextButton", {
		Size = UDim2.fromScale(0.3, 0.1), Position = UDim2.fromScale(0.35, 0.84),
		BackgroundColor3 = Color3.fromRGB(40, 150, 110), TextScaled = true, Font = Enum.Font.GothamBold,
		TextColor3 = Color3.new(1, 1, 1), Text = "Continue", ZIndex = 5,
	}, crash)
	new("UICorner", { CornerRadius = UDim.new(0.2, 0) }, cont)
	note.Visible = true

	table.insert(connections, cont.Activated:Connect(function()
		crash.Visible = false
		ack:FireServer("CurrencyCrashCourse")
	end))

	table.insert(connections, Remotes.event("TutorialStep").OnClientEvent:Connect(function(p: any)
		if type(p) ~= "table" then return end
		local id = tostring(p.id)
		if id == "CurrencyCrashCourse" then
			stepPanel.Visible = false
			crash.Visible = true
		else
			crash.Visible = false
			stepText.Text = tostring(p.text)
			stepPanel.Visible = true
			-- Non-crash-course steps advance server-side from gameplay; no ack needed.
		end
	end))

	script.Destroying:Once(function()
		for _, c in connections do c:Disconnect() end
		table.clear(connections)
		gui:Destroy()
	end)
end

return TutorialController
