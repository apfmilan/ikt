--!strict
-- Raid warning banner (Cut), tether fire list, Peaceful toggle.
local Players = game:GetService("Players")
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local TweenService = game:GetService("TweenService")

local Remotes = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))
local GameConfig = require(ReplicatedStorage.Shared:WaitForChild("GameConfig"))

local RaidController = {}

local function new(class: string, props: { [string]: any }, parent: Instance?): any
	local inst = Instance.new(class)
	for k, v in props do (inst :: any)[k] = v end
	inst.Parent = parent
	return inst
end

local started = false
local connections: { RBXScriptConnection } = {}

function RaidController.start()
	if started then return end
	started = true

	local localPlayer = Players.LocalPlayer
	local gui = new("ScreenGui", { Name = "Raid", ResetOnSpawn = false }, localPlayer:WaitForChild("PlayerGui"))

	-- Warning banner
	local banner = new("Frame", {
		Size = UDim2.fromScale(0.4, 0.09), Position = UDim2.fromScale(0.3, -0.12),
		BackgroundColor3 = Color3.fromRGB(170, 30, 30), Visible = false,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.15, 0) }, banner)
	local bannerText = new("TextLabel", {
		Size = UDim2.fromScale(0.68, 1), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.GothamBold, TextColor3 = Color3.new(1, 1, 1), Text = "",
	}, banner)
	local cutBtn = new("TextButton", {
		Size = UDim2.fromScale(0.28, 0.7), Position = UDim2.fromScale(0.7, 0.15),
		BackgroundColor3 = Color3.fromRGB(255, 200, 60), TextScaled = true, Font = Enum.Font.GothamBold,
		TextColor3 = Color3.new(0, 0, 0), Text = "Cut",
	}, banner)
	new("UICorner", { CornerRadius = UDim.new(0.2, 0) }, cutBtn)
	local shownPos = UDim2.fromScale(0.3, 0.12)
	local hiddenPos = UDim2.fromScale(0.3, -0.12)
	local info = TweenInfo.new(0.3, Enum.EasingStyle.Back, Enum.EasingDirection.Out)

	local tetherCut = Remotes.event("TetherCut")
	table.insert(connections, cutBtn.Activated:Connect(function() tetherCut:FireServer() end))

	local warnToken = 0
	local function hideBanner()
		warnToken += 1
		banner.Visible = false
		banner.Position = hiddenPos
	end
	table.insert(connections, Remotes.event("RaidWarning").OnClientEvent:Connect(function(p: any)
		if type(p) ~= "table" then return end
		if p.cleared then hideBanner() return end
		warnToken += 1
		local token = warnToken
		local attacker = Players:GetPlayerByUserId(tonumber(p.attackerUserId) or 0)
		local name = if attacker then attacker.DisplayName else "Someone"
		local secs = math.max(0, math.floor(tonumber(p.secondsLeft) or 0))
		banner.Visible = true
		TweenService:Create(banner, info, { Position = shownPos }):Play()
		task.spawn(function()
			while token == warnToken and secs >= 0 do
				bannerText.Text = `{name} is tethering you! {secs}s`
				secs -= 1
				task.wait(1)
			end
		end)
	end))

	-- Tether fire list
	local panel = new("Frame", {
		Size = UDim2.fromScale(0.16, 0.34), Position = UDim2.fromScale(0.01, 0.3),
		BackgroundColor3 = Color3.fromRGB(12, 28, 44), BackgroundTransparency = 0.2,
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.05, 0) }, panel)
	new("TextLabel", {
		Size = UDim2.fromScale(1, 0.1), BackgroundTransparency = 1, TextScaled = true,
		Font = Enum.Font.GothamBold, TextColor3 = Color3.new(1, 1, 1), Text = "Tether (nearby)",
	}, panel)
	local list = new("ScrollingFrame", {
		Size = UDim2.fromScale(1, 0.9), Position = UDim2.fromScale(0, 0.1), BackgroundTransparency = 1,
		CanvasSize = UDim2.new(), AutomaticCanvasSize = Enum.AutomaticSize.Y, ScrollBarThickness = 4,
		BorderSizePixel = 0,
	}, panel)
	new("UIListLayout", { Padding = UDim.new(0, 2) }, list)

	local tetherFire = Remotes.event("TetherFire")
	local rows: { [Player]: TextButton } = {}
	local range = GameConfig.Raid.TetherRange

	local function rootOf(p: Player): BasePart?
		local c = p.Character
		return c and c:FindFirstChild("HumanoidRootPart") :: BasePart?
	end

	local acc = 0
	table.insert(connections, game:GetService("RunService").Heartbeat:Connect(function(dt)
		acc += dt
		if acc < 0.5 then return end
		acc = 0
		local me = rootOf(localPlayer)
		for _, p in Players:GetPlayers() do
			if p == localPlayer then continue end
			local r = rootOf(p)
			local near = me ~= nil and r ~= nil and (r.Position - me.Position).Magnitude <= range
			local row = rows[p]
			if near and not row then
				local b = new("TextButton", {
					Size = UDim2.new(1, 0, 0, 28), BackgroundColor3 = Color3.fromRGB(150, 50, 50),
					TextScaled = true, Font = Enum.Font.Gotham, TextColor3 = Color3.new(1, 1, 1),
					Text = `Fire: {p.DisplayName}`,
				}, list)
				b.Activated:Connect(function() tetherFire:FireServer(p.UserId) end)
				rows[p] = b
			elseif not near and row then
				row:Destroy()
				rows[p] = nil
			end
		end
	end))
	table.insert(connections, Players.PlayerRemoving:Connect(function(p)
		local row = rows[p]
		if row then row:Destroy(); rows[p] = nil end
	end))

	-- Peaceful toggle
	local peaceful = false
	local peacefulBtn = new("TextButton", {
		Size = UDim2.fromScale(0.12, 0.05), Position = UDim2.fromScale(0.01, 0.65),
		BackgroundColor3 = Color3.fromRGB(60, 60, 70), TextScaled = true, Font = Enum.Font.GothamBold,
		TextColor3 = Color3.new(1, 1, 1), Text = "Peaceful: OFF",
	}, gui)
	new("UICorner", { CornerRadius = UDim.new(0.2, 0) }, peacefulBtn)
	local peacefulReq = Remotes.event("PeacefulToggle")
	table.insert(connections, peacefulBtn.Activated:Connect(function()
		peaceful = not peaceful
		peacefulBtn.Text = if peaceful then "Peaceful: ON" else "Peaceful: OFF"
		peacefulBtn.BackgroundColor3 = if peaceful then Color3.fromRGB(40, 140, 90) else Color3.fromRGB(60, 60, 70)
		peacefulReq:FireServer(peaceful)
	end))

	script.Destroying:Once(function()
		warnToken += 1
		for _, c in connections do c:Disconnect() end
		table.clear(connections)
		gui:Destroy()
	end)
end

return RaidController
