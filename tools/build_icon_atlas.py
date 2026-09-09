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
# El slot de cada icono es su indice en esta lista, asi que los nombres nuevos
# van SIEMPRE al final: intercalar uno corre a todos los de atras y les cambia
# el dibujo a las referencias que ya existen. Los cinco ultimos son de la barra
# de estado del shell (ver ux_vr.hsml).
names = ['home', 'settings', 'demos', 'app', 'about', 'scale', 'fire', 'target', 'range',
         'user', 'wifi', 'battery', 'bell', 'capture',
         'close', 'minimize', 'anchor',
         'search', 'add', 'edit', 'apps', 'chevron']
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
        elif name=='user':
            circle((66,24,126,84),11)
            # Hombros: media elipse por debajo del marco, para que corte recto
            # abajo en vez de cerrar en ovalo.
            d.arc((34,92,158,212),180,360,fill=white,width=12)
        elif name=='wifi':
            # Tres arcos concentricos desde un mismo centro y el punto abajo.
            for r in (80,54,28): d.arc((96-r,116-r,96+r,116+r),205,335,fill=white,width=12)
            d.ellipse((84,104,108,128),fill=white)
        elif name=='battery':
            d.rounded_rectangle((22,64,152,128),12,outline=white,width=10)
            d.rounded_rectangle((156,84,172,108),5,fill=white)
            d.rounded_rectangle((36,78,106,114),6,fill=white)
        elif name=='bell':
            # Cupula + faldon recto + badajo. La campana entera de una pieza
            # queda con la base curva y se lee como una gota.
            d.pieslice((50,30,142,122),180,360,fill=white)
            d.rectangle((50,76,142,128),fill=white)
            d.rounded_rectangle((34,128,158,146),9,fill=white)
            d.ellipse((84,150,108,174),fill=white)
        elif name=='capture':
            d.rounded_rectangle((22,56,170,160),16,outline=white,width=9)
            d.polygon([(64,58),(78,34),(114,34),(128,58)],fill=white)
            circle((70,78,122,130),10)
        elif name=='close':
            line([(58,58),(134,134)],14); line([(134,58),(58,134)],14)
        elif name=='minimize':
            line([(56,96),(136,96)],14)
        elif name=='anchor':
            # Chincheta: cabeza, cuerpo y punta.
            d.rounded_rectangle((62,30,130,52),8,fill=white)
            d.polygon([(78,52),(114,52),(122,108),(70,108)],fill=white)
            line([(96,108),(96,160)],11)
        elif name=='search':
            circle((34,34,126,126),13); line([(118,118),(158,158)],15)
        elif name=='add':
            line([(96,42),(96,150)],15); line([(42,96),(150,96)],15)
        elif name=='edit':
            d.polygon([(66,126),(122,70),(146,94),(90,150)],fill=white)
            d.polygon([(40,176),(66,126),(90,150)],fill=white)
        elif name=='apps':
            for cx in (54,96,138):
                for cy in (54,96,138): d.ellipse((cx-15,cy-15,cx+15,cy+15),fill=white)
        elif name=='chevron':
            line([(50,74),(96,124),(142,74)],16)
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
