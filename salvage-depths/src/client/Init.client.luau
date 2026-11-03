--!strict
-- Client entry point: starts every controller in Controllers/.
local controllers = script.Parent:WaitForChild("Controllers")

for _, module in controllers:GetChildren() do
	if not module:IsA("ModuleScript") then continue end
	local ok, controller = pcall(require, module)
	if not ok then
		warn(`[Init] failed to require {module.Name}: {controller}`)
		continue
	end
	local c = controller :: any
	if type(c) == "table" and type(c.start) == "function" then
		local startOk, err = pcall(c.start)
		if not startOk then
			warn(`[Init] {module.Name}.start failed: {err}`)
		end
	end
end
