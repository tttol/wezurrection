-- Managed by wezurrection: wait for the Nix-backed WezTerm configuration.
local wezterm = require 'wezterm'
local source = wezterm.home_dir .. '/.wezterm.lua'

-- The local loader is available before the separately mounted Nix store.
for attempt = 1, 600 do
    local file = io.open(source, 'r')
    if file then
        file:close()
        wezterm.add_to_config_reload_watch_list(source)
        return dofile(source)
    end
    wezterm.sleep_ms(100)
end

error('wezurrection: WezTerm configuration is still unavailable after 60 seconds: ' .. source)
