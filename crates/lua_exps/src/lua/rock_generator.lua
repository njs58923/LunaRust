-- rock_generator.lua
-- Genera un mesh de roca deterministico a partir de un seed.

local function make_rng(seed)
  local state = seed % 2147483648
  return {
    next = function(self)
      state = (state * 1664525 + 1013904223) % 2147483648
      return state / 2147483648
    end,
    range = function(self, min, max)
      return min + self:next() * (max - min)
    end,
    int = function(self, min, max)
      return math.floor(self:range(min, max + 1))
    end,
  }
end

local function sphere_points(rings, slices, radius)
  local verts = {}
  for i = 0, rings do
    local phi = math.pi * i / rings
    for j = 0, slices - 1 do
      local theta = 2 * math.pi * j / slices
      verts[#verts + 1] = {
        x = radius * math.sin(phi) * math.cos(theta),
        y = radius * math.cos(phi),
        z = radius * math.sin(phi) * math.sin(theta),
      }
    end
  end
  return verts
end

local function displace_verts(verts, rng, options)
  local amplitude  = options.amplitude  or 0.3
  local flatness_y = options.flatness_y or 1.0
  local scale_xz   = options.scale_xz   or 1.0
  for _, v in ipairs(verts) do
    local disp = rng:range(-amplitude, amplitude)
    local len  = math.sqrt(v.x^2 + v.y^2 + v.z^2)
    if len > 0 then
      v.x = (v.x + v.x / len * disp) * scale_xz
      v.y = (v.y + v.y / len * disp) * flatness_y
      v.z = (v.z + v.z / len * disp) * scale_xz
    end
  end
  return verts
end

local function build_faces(rings, slices)
  local faces = {}
  for i = 0, rings - 1 do
    for j = 0, slices - 1 do
      local a = i * slices + j + 1
      local b = i * slices + (j + 1) % slices + 1
      local c = (i + 1) * slices + j + 1
      local d = (i + 1) * slices + (j + 1) % slices + 1
      faces[#faces + 1] = { a, b, c }
      faces[#faces + 1] = { b, d, c }
    end
  end
  return faces
end

local function generate_rock(seed, options)
  options = options or {}
  local rng = make_rng(seed)
  local rings     = options.rings     or rng:int(5, 10)
  local slices    = options.slices    or rng:int(6, 12)
  local radius    = options.radius    or rng:range(0.8, 1.5)
  local amplitude = options.amplitude or rng:range(0.15, 0.4)
  local flatness  = options.flatness  or rng:range(0.5, 1.0)
  local scale_xz  = options.scale_xz  or rng:range(0.8, 1.2)
  local verts = sphere_points(rings, slices, radius)
  displace_verts(verts, rng, {
    amplitude  = amplitude,
    flatness_y = flatness,
    scale_xz   = scale_xz,
  })
  local faces = build_faces(rings, slices)
  return {
    vertices = verts,
    faces = faces,
    meta = { seed = seed, rings = rings, slices = slices },
  }
end

_G.generate_rock = generate_rock
return generate_rock
