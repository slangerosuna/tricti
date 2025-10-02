if exists("g:loaded_tricti_runtime")
  finish
endif
let g:loaded_tricti_runtime = 1

if has('nvim')
  lua << EOF
  local group = vim.api.nvim_create_augroup("TriCTIFiletype", { clear = true })
  vim.api.nvim_create_autocmd({ "BufReadPost", "BufNewFile", "BufEnter" }, {
    group = group,
    pattern = "*.tri",
    callback = function(event)
      vim.bo[event.buf].filetype = "tricti"
    end,
  })

  if vim.filetype and vim.filetype.add then
    vim.filetype.add({
      extension = {
        tri = "tricti",
      },
    })
  end

  vim.schedule(function()
    local buf = vim.api.nvim_get_current_buf()
    if buf == 0 then
      return
    end
    local name = vim.api.nvim_buf_get_name(buf)
    if name:match("%.tri$") then
      vim.bo[buf].filetype = "tricti"
    end
  end)
EOF
else
  augroup tricti_filetype_runtime
    autocmd!
    autocmd BufRead,BufNewFile *.tri setfiletype tricti
  augroup END
endif
