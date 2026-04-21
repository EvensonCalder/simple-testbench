local function has_key(body, key)
  return body:match('"' .. key .. '"%s*:') ~= nil
end

local function count_object_keys(body)
  local count = 0
  for _ in body:gmatch('"[^"]+"%s*:') do
    count = count + 1
  end
  return count
end

return function(processed_output)
  local body = tostring(processed_output or "")
  body = body:gsub("^%s+", "")
  body = body:gsub("%s+$", "")

  local ok = body:sub(1, 1) == "{" and body:sub(-1) == "}"
    and has_key(body, "todo")
    and has_key(body, "time")
    and has_key(body, "location")
    and count_object_keys(body) == 3

  if ok then
    return 100
  end

  return 0
end
