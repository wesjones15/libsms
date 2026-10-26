"""Local viewer proof inputs only; never package the exported game assets.

Requires UnityPy. Arguments: installed game's StandaloneWindows64 bundle folder,
then an ignored output folder. Targets the Unity 2021.3 DLC4 SurvivorPod.
"""
from pathlib import Path
import struct, json, sys, lz4.block, UnityPy
base=Path(sys.argv[1])
out=Path(sys.argv[2]); out.mkdir(parents=True,exist_ok=True)
def cabnames(path):
    with path.open('rb') as f:
        def z():
            b=bytearray()
            while (c:=f.read(1)) not in (b'',b'\0'): b.extend(c)
            return b.decode()
        if z()!='UnityFS': return []
        version=struct.unpack('>I',f.read(4))[0]; z(); z()
        size,cs,us,flags=struct.unpack('>QIII',f.read(20))
        if version>=7: f.seek((f.tell()+15)//16*16)
        if flags&128: f.seek(size-cs)
        data=f.read(cs)
        if flags&63 in (2,3): data=lz4.block.decompress(data,uncompressed_size=us)
        n=struct.unpack_from('>I',data,16)[0]; at=20+n*10
        count=struct.unpack_from('>I',data,at)[0]; at+=4
        names=[]
        for _ in range(count):
            at+=20; end=data.index(0,at); names.append(data[at:end].decode()); at=end+1
        return names
index={name:p for p in base.glob('*.bundle') for name in cabnames(p)}
env=UnityPy.load(*map(str,base.glob('ror2-base-survivorpod_*assets_all*.bundle')))
loaded=set()
def get(pid):
    return next(o for o in env.objects if o.path_id==pid)
def deps(obj, ids):
    for i in ids:
        name=obj.assets_file.externals[i-1].path.split('/')[-1]
        p=index[name]
        if p not in loaded: env.load_file(str(p)); loaded.add(p); print('dependency',p.name)
mat=get(-6514494002267390535)
deps(mat,[2,3,4,5,6])
# The emitter's mesh is external 3 in the prefab, not in the material file.
renderer=get(-4483200324324671225)
deps(renderer,[3])
mesh=get(-6476590250452040025).read()
(out/'flame.obj').write_text(mesh.export())
for name,value in mat.read_typetree()['m_SavedProperties']['m_TexEnvs']:
    pid=value['m_Texture']['m_PathID']; obj=get(pid); data=obj.read()
    data.image.save(out/(name+'.png')); print(name,pid,data.m_Name)
shader=get(-658352061839041481).read()
try: (out/'shader.txt').write_text(shader.export())
except Exception as e: print('shader export',e)
for pid,label in [(-470276994286075641,'flames'),(-4483200324324671225,'renderer')]:
    (out/(label+'.json')).write_text(json.dumps(get(pid).read_typetree(),indent=2))
print('mesh',mesh.m_Name,'vertices',mesh.m_VertexData.m_VertexCount)
# Report exact hierarchy and enabled modules without huge typetree dumps.
for obj in env.objects:
    if obj.type.name=='Transform' and obj.assets_file==renderer.assets_file:
        d=obj.read_typetree(); name=get(d['m_GameObject']['m_PathID']).read().m_Name
        if name in ('Flames','FireDonut','TravelParent','Base','Dust, Directional'):
            print('transform',name,obj.path_id,d['m_Father'],d['m_LocalPosition'],d['m_LocalScale'])
flames=get(-470276994286075641).read_typetree()
for key,value in flames.items():
    if isinstance(value,dict) and value.get('enabled'): print('module',key,json.dumps(value)[:1600])
