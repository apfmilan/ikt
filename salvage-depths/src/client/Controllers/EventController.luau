--!strict
-- Trench Surge: announce banner + countdown, pressure meter, top 3, DeliverCore.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local TweenService = game:GetService("TweenService")

local Remotes = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))

local EventController = {}

local function new(class: string, props: { [string]: any }, parent: Instance?): any
	local inst = Instance.new(class)
	for k, v in props do (inst :: any)[k] = v end
	inst.Parent = parent
	return inst
end

local started = false
local connections: { RBXScriptConnection } = {}

function EventController.start()
	if started then return end
	started = true

	local gui = new("ScreenGui", { Name = "Event", ResetOnSpawn = false }, Players.LocalPlayer:WaitForChild("PlayerGui"))

	local banner = new("TextLabel", {
		Size = UDim2.fromScale(0.4, 0.06), Position = UDim2.fromScale(0.3, 0.01),
		BackgroundColor3 = Color3.fromRGB(90, 40, 140), TextScaled = true, Font = Enum.Font.GothamBold,
		TextColor3 = Color3.new(1, 1, 1), Text = "", Visible = false,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.2, 0) }, banner)

	local panel = new("Frame", {
		Size = UDim2.fromScale(0.2, 0.26), Position = UDim2.fromScale(0.79, 0.3),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), BackgroundTransparency = 0.2, Visible = false,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.05, 0) }, panel)
	new("TextLabel", {
		Size = UDim2.fromScale(1, 0.12), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.GothamBold, TextColor3 = Color3.new(1, 1, 1), Text = "Pressure",
	}, panel)
	local meterBack = new("Frame", {
		Size = UDim2.fromScale(0.9, 0.1), Position = UDim2.fromScale(0.05, 0.14),
		BackgroundColor3 = Color3.fromRGB(10, 10, 20), BorderSizePixel = 0,
	}, panel)
	new("UICorner", { CornerRadius = UDim.new(0.5, 0) }, meterBack)
	local meterFill = new("Frame", {
		Size = UDim2.fromScale(0, 1), BackgroundColor3 = Color3.fromRGB(240, 120, 60), BorderSizePixel = 0,
	}, meterBack)
	new("UICorner", { CornerRadius = UDim.new(0.5, 0) }, meterFill)
	local yourLabel = new("TextLabel", {
		Size = UDim2.fromScale(1, 0.1), Position = UDim2.fromScale(0, 0.26), BackgroundTransparency = 1,
		TextScaled = true, Font = Enum.Font.GothamMedium, TextColor3 = Color3.new(1, 1, 1), Text = "Your score: 0",
	}, panel)
	local topLabels: { TextLabel } = {}
	for i = 1, 3 do
		topLabels[i] = new("TextLabel", {
			Size = UDim2.fromScale(0.9, 0.09), Position = UDim2.fromScale(0.05, 0.38 + (i - 1) * 0.1),
			BackgroundTransparency = 1, TextScaled = true, Font = Enum.Font.Gotham,
			TextColor3 = Color3.fromRGB(255, 230, 150), TextXAlignment = Enum.TextXAlignment.Left, Text = "",
		}, panel)
	end
	local deliver = new("TextButton", {
		Size = UDim2.fromScale(0.9, 0.2), Position = UDim2.fromScale(0.05, 0.74),
		BackgroundColor3 = Color3.fromRGB(200, 110, 30), TextScaled = true, Font = Enum.Font.GothamBold,
		TextColor3 = Color3.new(1, 1, 1), Text = "Deliver Core", Visible = false,
	}, panel)
	new("UICorner", { CornerRadius = UDim.new(0.15, 0) }, deliver)
	local deliverReq = Remotes.event("DeliverCore")
	table.insert(connections, deliver.Activated:Connect(function() deliverReq:FireServer() end))

	local token = 0
	local meterInfo = TweenInfo.new(0.3)

	table.insert(connections, Remotes.event("EventAnnounce").OnClientEvent:Connect(function(p: any)
		if type(p) ~= "table" then return end
		token += 1
		local my = token
		if p.ended then
			banner.Visible = false
			panel.Visible = false
			deliver.Visible = false
			return
		end
		local startsIn = math.max(0, math.floor(tonumber(p.startsIn) or 0))
		local duration = math.max(0, math.floor(tonumber(p.duration) or 0))
		local name = tostring(p.name)
		banner.Visible = true
		task.spawn(function()
			while my == token and startsIn > 0 do
				banner.Text = `{name} starts in {startsIn}s`
				startsIn -= 1
				task.wait(1)
			end
			if my ~= token then return end
			panel.Visible = true
			deliver.Visible = true
			while my == token and duration > 0 do
				banner.Text = `{name}: {duration}s left`
				duration -= 1
				task.wait(1)
			end
			if my == token then
				banner.Visible = false
				deliver.Visible = false
			end
		end)
	end))

	table.insert(connections, Remotes.event("EventScore").OnClientEvent:Connect(function(p: any)
		if type(p) ~= "table" then return end
		panel.Visible = true
		TweenService:Create(meterFill, meterInfo, { Size = UDim2.fromScale(math.clamp((tonumber(p.meter) or 0) / 100, 0, 1), 1) }):Play()
		yourLabel.Text = `Your score: {math.floor(tonumber(p.yourScore) or 0)}`
		local top = if type(p.top) == "table" then p.top else {}
		for i = 1, 3 do
			local e = top[i]
			topLabels[i].Text = if e then `{i}. {tostring(e.name)} - {math.floor(tonumber(e.score) or 0)}` else ""
		end
	end))

	script.Destroying:Once(function()
		token += 1
		for _, c in connections do c:Disconnect() end
		table.clear(connections)
		gui:Destroy()
	end)
end

return EventController
