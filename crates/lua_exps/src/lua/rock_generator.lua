-- rock_generator.lua
-- Genera meshes de roca deterministicos a partir de un seed.
-- Expone:
--   generate(seed, options)       -> { vertices=flat, faces=flat, meta }
--   generate_batch(seeds, options) -> { vertices=flat, faces=flat,
--                                       vertex_counts, face_counts }
-- vertices: {x,y,z, x,y,z, ...}
-- faces:    {a,b,c, a,b,c, ...} (1-indexed, local a cada roca en batch).

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
local move = table.move

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

local function generate_batch(seeds, options)
  local out_v, out_f = {}, {}
  local v_counts, f_counts = {}, {}
  local v_off, f_off = 0, 0
  for i = 1, #seeds do
    local r = generate(seeds[i], options)
    local rv, rf = r.vertices, r.faces
    local nv, nf = #rv, #rf
    move(rv, 1, nv, v_off + 1, out_v)
    move(rf, 1, nf, f_off + 1, out_f)
    v_counts[i] = nv / 3
    f_counts[i] = nf / 3
    v_off = v_off + nv
    f_off = f_off + nf
  end
  return {
    vertices = out_v,
    faces = out_f,
    vertex_counts = v_counts,
    face_counts = f_counts,
  }
end

_G.generate = generate
_G.generate_batch = generate_batch
return generate
