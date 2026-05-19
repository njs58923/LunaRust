-- rock_generator.lua
-- Genera meshes de roca deterministicos a partir de un seed.
-- Expone:
--   generate(seed, options) -> { vertices=flat, faces=flat, meta }
-- vertices: {x,y,z, x,y,z, ...}
-- faces:    {a,b,c, a,b,c, ...} (1-indexed)

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

local sin, cos, sqrt, pi = math.sin, math.cos, math.sqrt, math.pi

local function generate(seed, options)
  options = options or {}
  local rng = make_rng(seed)
  local rings     = options.rings     or rng:int(5, 10)
  local slices    = options.slices    or rng:int(6, 12)
  local radius    = options.radius    or rng:range(0.8, 1.5)
  local amplitude = options.amplitude or rng:range(0.15, 0.4)
  local flatness  = options.flatness  or rng:range(0.5, 1.0)
  local scale_xz  = options.scale_xz  or rng:range(0.8, 1.2)

  local verts = {}
  local n = 0
  for i = 0, rings do
    local phi = pi * i / rings
    local sphi = sin(phi)
    local cphi = cos(phi)
    for j = 0, slices - 1 do
      local theta = 2 * pi * j / slices
      local x = radius * sphi * cos(theta)
      local y = radius * cphi
      local z = radius * sphi * sin(theta)
      local disp = rng:range(-amplitude, amplitude)
      local len = sqrt(x * x + y * y + z * z)
      if len > 0 then
        x = (x + x / len * disp) * scale_xz
        y = (y + y / len * disp) * flatness
        z = (z + z / len * disp) * scale_xz
      end
      verts[n * 3 + 1] = x
      verts[n * 3 + 2] = y
      verts[n * 3 + 3] = z
      n = n + 1
    end
  end

  local faces = {}
  local f = 0
  for i = 0, rings - 1 do
    for j = 0, slices - 1 do
      local a = i * slices + j + 1
      local b = i * slices + (j + 1) % slices + 1
      local c = (i + 1) * slices + j + 1
      local d = (i + 1) * slices + (j + 1) % slices + 1
      faces[f + 1] = a
      faces[f + 2] = b
      faces[f + 3] = c
      faces[f + 4] = b
      faces[f + 5] = d
      faces[f + 6] = c
      f = f + 6
    end
  end

  return {
    vertices = verts,
    faces = faces,
    meta = { seed = seed, rings = rings, slices = slices },
  }
end

_G.generate = generate
return generate
