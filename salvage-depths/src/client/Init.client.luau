--!strict
-- Client entry point: starts every controller in Controllers/.
local ReplicatedStorage = game:GetService("ReplicatedStorage")
local Remotes = require(ReplicatedStorage:WaitForChild("Shared"):WaitForChild("Remotes"))

local controllers = script.Parent:WaitForChild("Controllers")

local pending = 0
local scheduled = false

local function finish()
	pending -= 1
	if scheduled and pending == 0 then
		Remotes.event("ClientReady"):FireServer()
	end
end

for _, module in controllers:GetChildren() do
	if not module:IsA("ModuleScript") then continue end
	pending += 1
	task.spawn(function()
		local ok, controller = pcall(require, module)
		if not ok then
			warn(`[Init] failed to require {module.Name}: {controller}`)
		else
			local c = controller :: any
			if type(c) == "table" and type(c.start) == "function" then
				local startOk, err = pcall(c.start)
				if not startOk then
					warn(`[Init] {module.Name}.start failed: {err}`)
				end
			end
		end
		finish()
	end)
end

scheduled = true
if pending == 0 then
	Remotes.event("ClientReady"):FireServer()
end
