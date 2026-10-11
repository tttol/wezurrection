-- Run with: lua tests/startup_config_test.lua /path/to/empty/test-directory
local home = assert(arg[1], 'a temporary test directory is required')
local source = home .. '/.wezterm.lua'
local sleeps = 0
local watched = nil
local config_text = 'return { font_size = 15, color_scheme = "Tokyo Night" }'

package.preload.wezterm = function()
    return {
        home_dir = home,
        sleep_ms = function(milliseconds)
            assert(milliseconds == 100)
            sleeps = sleeps + 1
            if sleeps == 3 then
                local file = assert(io.open(source, 'w'))
                file:write(config_text)
                file:close()
            end
        end,
        add_to_config_reload_watch_list = function(path)
            watched = path
        end,
    }
end

-- Given: the configuration target is initially unavailable.
-- When: the simulated volume becomes available on the third poll.
local config = dofile('src/startup_config.lua')

-- Then: the settings are loaded before returning, without a reload event.
assert(config.font_size == 15 and config.color_scheme == 'Tokyo Night')
assert(sleeps == 3)
assert(watched == source)

-- Given: the configuration is already readable on a later evaluation.
sleeps = 0

-- When
local subsequent = dofile('src/startup_config.lua')

-- Then: it loads immediately and preserves the settings.
assert(subsequent.font_size == 15 and subsequent.color_scheme == 'Tokyo Night')
assert(sleeps == 0)

-- Given: the target never becomes available.
assert(os.remove(source))
local wezterm = require 'wezterm'
wezterm.sleep_ms = function() sleeps = sleeps + 1 end

-- When
local success, error_message = pcall(dofile, 'src/startup_config.lua')

-- Then: failure is explicit rather than silently returning an empty config.
assert(not success)
assert(sleeps == 600)
assert(error_message:match('still unavailable after 60 seconds'))

print('Startup config tests passed: delayed mount, immediate load, timeout')
