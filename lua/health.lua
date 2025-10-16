-- The nvim health checks need a physical file, so create a minimal one and add it to the runtime path
local base_dir = vim.fn.tempname()
local plugin_dir = vim.fs.joinpath(base_dir, "lua", "neovide")
local health_lua = vim.fs.joinpath(plugin_dir, "health.lua")
local contents = {
    "return { check = neovide.health }",
}
vim.opt.rtp:append(base_dir)
vim.fn.mkdir(plugin_dir, "p")
vim.fn.writefile(contents, health_lua)

neovide.private.health = {
    nerd_font_fallback = "FiraCode Nerd Font",
    nerd_font_primary = false,
    failed_glyphs = {"A", "B", "C"}
}

neovide.health = function()
    local faq = "See: https://neovide.dev/faq.html"
    local h = neovide.private.health
    vim.health.start("fonts")
    local font = h["nerd_font_fallback"]
    if font then
        vim.health.ok("A Nerd Font fallback font is installed (" .. font .. ")")
        if h["nerd_font_primary"] then
            vim.health.ok("The Nerd Font is the primary font")
        else
            vim.health.warn(
                "The primary font is not a Nerd Font",
                {
                    "The primary font should be a Nerd Font, otherwise some glyphs might render with gaps or wrong proportions",
                    faq,
                }
            )
        end
    else
        vim.health.warn(
            "A Nerd Fonts fallback font is not installed",
            { "Many plugins assume that a Nerd Font is setup", faq }
        )
    end
    if h.failed_glyphs then
        vim.health.error("Some glyph clusters failed to render correctly", {"Ensure that all required fonts are installed", faq})
        vim.health.info("Recently failed glyph clusters (the list might be incomplete")
        vim.health.info(vim.inspect(h.failed_glyphs))
    else
    end

    -- make sure setup function parameters are ok
    vim.health.ok("Setup is correct")
    vim.health.warn("A nerd font is recommended", { "advice1", "advice" })
end
