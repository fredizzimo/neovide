local function rpcnotify(method, ...)
    vim.rpcnotify(vim.g.neovide_channel_id, method, ...)
end

---@type table<integer, boolean>
local LOADED_IMAGES = {}
local next_placement_id = 1

if vim.ui.img then
    vim.ui.img.providers['neovide'] = vim.ui.img.providers.new({
        --- @param self vim.ui.img.Provider
        on_unload = function(self)
        end,
        --- @param self vim.ui.img.Provider
        on_load = function(self)
        end,
        --- @param self vim.ui.img.Provider
        --- @param img vim.ui.Image
        --- @param opts? vim.ui.img.Opts
        --- @return integer
        on_show = function(self, img, opts)
            if not LOADED_IMAGES[img.id] then
                rpcnotify("neovide.img.upload", {
                    img = img,
                    more_chunks = false,
                    base64 = false,
                })
                LOADED_IMAGES[img.id] = true
            end
            local placement_id = next_placement_id
            rpcnotify("neovide.img.show", {
                image_id = img.id,
                placement_id = placement_id,
                opts = opts,
            })
            next_placement_id = next_placement_id + 1
            return next_placement_id
        end,
        --- @param self vim.ui.img.Provider
        --- @param ids integer[]
        on_hide = function(self, ids)
        end,
        --- @param self vim.ui.img.Provider
        --- @param id integer
        --- @param opts? vim.ui.img.Opts
        --- @return integer
        on_update = function(self, id, opts)
            return id
        end
    })
end

neovide.img = {}

local function get_crop(kitty_image)
    local x = kitty_image.x or 0
    local y = kitty_image.y or 0
    local w = kitty_image.w or 0
    local h = kitty_image.h or 0
    if x or y or w or h then
        local x1 = x
        local y1 = y
        local x2 = x1 + w
        local y2 = y1 + h
        return {
                pos1 = {
                    x = x1,
                    y = y1,
                    unit = "pixel",
                },
                pos2 = {
                    x = x2,
                    y = y2,
                    unit = "pixel",
                }
            }
    else
        return nil
    end
end

local function get_size(kitty_image)
    local c = kitty_image.c or 0
    local r = kitty_image.r or 0
    if c or r then
        return {
            width = c,
            height = r,
            unit = "cell"
        }
    else
        return nil
    end
end

neovide.img.kitty_image = function(data)
    if not data.a or data.a == "t" then
        local img = {
            id = data.i or 0,
            bytes = data.data,
            filename = "",
        }
        local more_chunks = (data.m or 0) == 1
        rpcnotify("neovide.img.upload", {
            img = img,
            more_chunks = more_chunks,
            base64 = true,
        })
    elseif data.p then
        local opts = {
            relative = "placement",
            crop = get_crop(data),
            pos = nil,
            size = get_size(data),
            z = data.z or 0,
        }
        local image_id = data.i
        local placement_id = data.p
        rpcnotify("neovide.img.show", {
            image_id = image_id,
            placement_id = placement_id,
            opts = opts,
        })
    end
end


local M = {}
