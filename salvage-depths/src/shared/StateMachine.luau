--!strict
export type State = { onEnter: (() -> ())?, onExit: (() -> ())? }
export type Machine = { states: { [string]: State }, current: string, go: (Machine, string) -> () }

local StateMachine = {}
StateMachine.__index = StateMachine

function StateMachine.new(states: { [string]: State }, initial: string): Machine
	local self = setmetatable({ states = states, current = initial }, StateMachine)
	local s = states[initial]
	if s and s.onEnter then s.onEnter() end
	return (self :: any) :: Machine
end

function StateMachine.go(self: Machine, nextName: string)
	local new = self.states[nextName]
	assert(new, "Unknown state " .. nextName)
	local old = self.states[self.current]
	if old and old.onExit then old.onExit() end
	self.current = nextName
	if new.onEnter then new.onEnter() end
end

return StateMachine
