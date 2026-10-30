import json, subprocess, collections, sys
from PIL import Image, ImageDraw, ImageFont
sys.setrecursionlimit(20000)

APPS = ['navop','navop-shell','navop-data']
meta = json.loads(subprocess.run(['cargo','metadata','--no-deps','--format-version','1'],capture_output=True,text=True).stdout)
ws = {p['name'] for p in meta['packages']}

def edges(product):
    out = subprocess.run(['cargo','tree','-p',product,'-e','normal','--prefix','depth'],capture_output=True,text=True).stdout
    stack, es = {}, set()
    for line in out.splitlines():
        if not line.strip(): continue
        d=0
        while d < len(line) and line[d].isdigit(): d+=1
        depth=int(line[:d]) if d else 0
        name=line[d:].strip().split(' v')[0].split(' ')[0]
        stack[depth]=name
        if depth>0 and name in ws and stack.get(depth-1) in ws:
            es.add((stack[depth-1],name))
    return es

per={p:edges(p) for p in APPS}
membership=collections.defaultdict(set)
for p in APPS:
    for n in {x for e in per[p] for x in e}|{p}:
        membership[n].add(p)
nodes=set(membership)
union=set().union(*per.values())

preds=collections.defaultdict(set)
for a,b in union:
    preds[b].add(a)

state={}
def level(n, seen=()):
    if n in state: return state[n]
    if n in seen: return 0
    if n in APPS: state[n]=0; return 0
    ps=preds.get(n,set())
    lv=1+max((level(p, seen+(n,)) for p in ps), default=-1) if ps else 0
    state[n]=max(lv,0); return state[n]
for n in nodes: level(n)

levels=collections.defaultdict(list)
for n in nodes: levels[state[n]].append(n)

def color(n):
    if n in APPS: return (62,74,92), (255,255,255)
    m=membership[n]
    if len(m)==3: return (203,232,203),(20,20,20)
    if m=={'navop','navop-shell'}: return (255,213,160),(20,20,20)
    if m=={'navop','navop-data'}: return (186,214,252),(20,20,20)
    return (228,228,228),(20,20,20)

fp='/System/Library/Fonts/Supplemental/Arial Unicode.ttf'
F=lambda s: ImageFont.truetype(fp,s)
NW,NH,GX,GY,PAD=152,22,26,10,70
maxlvl=max(len(v) for v in levels.values())
W=PAD*2+maxlvl*(NW+GX)
H=PAD*2+(max(levels)+1)*(NH+GY)+110
img=Image.new('RGB',(W,H),(255,255,255)); d=ImageDraw.Draw(img)
mid={}
order_rank=lambda n:(0 if n in APPS else 1, {'all':0,'shell':1,'data':2,'full':3}[('all' if len(membership[n])==3 else 'shell' if membership[n]=={'navop','navop-shell'} else 'data' if membership[n]=={'navop','navop-data'} else 'full')], n)
for lv,ns in sorted(levels.items()):
    ns=sorted(ns,key=order_rank)
    tw=len(ns)*(NW+GX)-GX; x0=(W-tw)//2; y=PAD+lv*(NH+GY)
    for i,n in enumerate(ns):
        x=x0+i*(NW+GX); cx=x+NW/2; cy=y+NH/2
        mid[n]=(cx,cy,x,y)
for a,b in union:
    if a in mid and b in mid:
        ax,ay,_,_=mid[a]; bx,by,_,_=mid[b]
        d.line([(ax,ay+NH/2-3),(bx,by-NH/2+2)],fill=(206,206,206),width=1)
for n,(cx,cy,x,y) in mid.items():
    fill,fg=color(n)
    d.rounded_rectangle([x,y,x+NW,y+NH],radius=4,fill=fill,outline=(110,110,110))
    d.text((x+5,y+5),n,font=F(12),fill=fg)
lx,ly=40,H-64
d.text((lx,ly-22),'navop 三产品依赖图（仅工作区内部 crate；node 颜色 = 出现在哪些产品的 cargo tree 中）',font=F(14),fill=(0,0,0))
import math
def swatch(x,y,c): d.rounded_rectangle([x,y,x+24,y+15],radius=3,fill=c,outline=(110,110,110))
legend=[('应用入口',(62,74,92)),('三产品共有',(203,232,203)),('全量 + 终端（shell）',(255,213,160)),('全量 + 数据（data）',(186,214,252)),('仅全量',(228,228,228))]
x=lx
for label,c in legend:
    swatch(x,ly,c); d.text((x+32,ly),label,font=F(13),fill=(0,0,0)); x+=270
img.save("docs/design/navop-product-dependency-graph.png")
print('saved',W,H,'nodes',len(nodes),'edges',len(union))
