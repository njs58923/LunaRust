"""Deterministic 32-slot icon atlas. Python + Pillow. Optional --input PNG directory.

Stable slots, transparent gutters and a content fingerprint: unchanged inputs do
not rewrite the atlas. Runtime instances change texture-region, not the PNG.
"""
from pathlib import Path
import argparse, hashlib, json, math
from PIL import Image, ImageDraw

parser = argparse.ArgumentParser()
parser.add_argument('--input', type=Path)
parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1]/'crates/luna/assets/icons')
args = parser.parse_args()
names = ['home', 'settings', 'demos', 'app', 'about', 'scale', 'fire', 'target', 'range']
icons = []
if args.input:
    for path in sorted(args.input.glob('*.png')):
        with Image.open(path) as image:
            icons.append((path.stem, image.convert('RGBA')))
else:
    for name in names:
        im = Image.new('RGBA', (192,192)); d=ImageDraw.Draw(im); white=(255,255,255,255)
        def line(points, width=10): d.line(points, fill=white, width=width, joint='curve')
        def circle(box, width=10): d.ellipse(box, outline=white, width=width)
        if name=='home':
            line([(25,85),(96,28),(167,85)]); line([(45,75),(45,160),(147,160),(147,75)])
            line([(79,160),(79,113),(113,113),(113,160)])
        elif name=='settings':
            circle((52,52,140,140)); circle((80,80,112,112),8)
            for i in range(8):
                a=i*math.tau/8; line([(96+math.cos(a)*43,96+math.sin(a)*43),(96+math.cos(a)*69,96+math.sin(a)*69)],17)
        elif name=='demos':
            for x,y in [(38,38),(106,38),(38,106),(106,106)]: d.rounded_rectangle((x,y,x+48,y+48),8,fill=white)
        elif name=='app':
            d.rounded_rectangle((27,40,165,154),12,outline=white,width=9); line([(30,72),(162,72)],7)
            for x in [44,61,78]: d.ellipse((x,53,x+6,59),fill=white)
            line([(52,104),(70,119),(53,132)],7); line([(88,133),(121,133)],7)
        elif name=='about':
            circle((28,28,164,164),9); d.ellipse((87,56,105,74),fill=white); line([(89,90),(100,90),(100,136)],11); line([(83,138),(115,138)],9)
        elif name=='scale':
            for sx,sy in [(-1,-1),(1,-1),(-1,1),(1,1)]:
                line([(96+sx*14,96+sy*14),(96+sx*58,96+sy*58)],9)
                line([(96+sx*24,96+sy*58),(96+sx*58,96+sy*58),(96+sx*58,96+sy*24)],9)
        elif name=='fire':
            d.polygon([(92,22),(124,69),(118,85),(145,65),(156,110),(144,143),(119,163),(79,162),(49,140),(38,113),(52,76),(59,104),(78,75)],fill=white)
            d.polygon([(96,95),(118,133),(107,150),(86,149),(77,133)],fill=(0,0,0,0))
        elif name=='target':
            circle((24,24,168,168),9);circle((53,53,139,139),9);d.ellipse((82,82,110,110),fill=white)
        elif name=='range':
            circle((51,51,141,141),8)
            for a in range(4):
                t=a*math.pi/2;line([(96+math.cos(t)*24,96+math.sin(t)*24),(96+math.cos(t)*75,96+math.sin(t)*75)],8)
        icons.append((name,im))
if not icons or len(icons)>32: raise SystemExit('Expected 1–32 PNG icons')
cell=96; atlas=Image.new('RGBA',(cell*8,cell*4)); manifest={}
for i,(name,icon) in enumerate(icons):
    icon.thumbnail((72,72),Image.Resampling.LANCZOS)
    x,y=(i%8)*cell,(i//8)*cell
    atlas.alpha_composite(icon,(x+(cell-icon.width)//2,y+(cell-icon.height)//2))
    manifest[name]={'region':[i%8/8,i//8/4,1/8,1/4]}
fingerprint=hashlib.sha256(atlas.tobytes()+json.dumps(manifest,sort_keys=True).encode()).hexdigest()
args.output.mkdir(parents=True,exist_ok=True)
metadata=args.output/'menu.json'
if metadata.exists() and (args.output/'menu.png').exists() and json.loads(metadata.read_text()).get('fingerprint')==fingerprint:
    print('Atlas unchanged')
else:
    atlas.save(args.output/'menu.png',optimize=True)
    metadata.write_text(json.dumps({'fingerprint':fingerprint,'size':list(atlas.size),'icons':manifest},indent=2)+'\n')
    print(f'Atlas: {len(icons)} icons, {atlas.size}, {fingerprint[:12]}')
