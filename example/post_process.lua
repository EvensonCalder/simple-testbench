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

return function(raw_output)
  local body = tostring(raw_output or "")
  body = body:gsub("^%s+", "")
  body = body:gsub("%s+$", "")

  local looks_like_json_object = body:sub(1, 1) == "{" and body:sub(-1) == "}"
  local valid_shape = looks_like_json_object
    and has_key(body, "todo")
    and has_key(body, "time")
    and has_key(body, "location")
    and count_object_keys(body) == 3

  if not valid_shape then
    return {
      retry = true,
      max_retry = 3,
      output = body,
    }
  end

  return {
    output = body,
  }
end
