"""探索（conditions.md で事前に決めていない・判定に使わない）: 要求と揃えた対照の直前の最後のキーの種類と、直前の休止 P の分布。

使い方: python3 -I explore_lastkey.py <scripts_dir>   (dwell 100 の中間を読む)
"""
# 探索（事前に決めていない）: 要求と揃えた対照の直前の最後のキーの種類と、直前の休止 P の分布
import sys, os, random, collections, bisect, json
sys.path.insert(0, sys.argv[1])
import analyze as A
wd = os.path.expanduser('~/gse-public-data/coauthor/work_dwell100')
raw = os.path.expanduser('~/gse-public-data/coauthor/raw')
writers = A.load_writers([raw+'/metadata_creative.csv', raw+'/metadata_argumentative.csv'])
import glob
cat = {}
def lastcat(sid, T):
    if sid not in cat:
        ks=[]
        for l in open(f'{wd}/{sid}.ndjson'):
            r=json.loads(l)
            if r.get('type')=='key' and r['press']: ks.append((r['t'],r['category']))
        cat[sid]=ks
    ks=cat[sid]; i=bisect.bisect_left(ks,(T,''))
    return ks[i-1][1] if i>0 else None
by_w={}
for p in sorted(glob.glob(wd+'/*_events.json')):
    sid=os.path.basename(p)[:-12]
    if sid in writers: by_w.setdefault(writers[sid],[]).append(A.Session(wd,sid))
rng=random.Random(7); cr=collections.Counter(); cc=collections.Counter(); Ps=[]
for w,sess in by_w.items():
    reqs=[(s,T,T-s.last_press_before(T)) for s in sess for T in s.req if s.req_ok(T)]
    if len(reqs)<5: continue
    gaps=[(s,a,b-a) for s in sess for a,b in zip(s.press,s.press[1:])]
    for s,T,P in reqs:
        Ps.append(P); cr[lastcat(s.sid,T)]+=1
        c=[(g[0],g[1]+P) for g in gaps if g[2]>P and g[0].ctrl_ok(g[1]+P)]
        for s2,T2 in rng.sample(c,min(5,len(c))): cc[lastcat(s2.sid,T2)]+=1
nr=sum(cr.values()); nc=sum(cc.values())
for k in sorted(set(cr)|set(cc), key=lambda k:-cr[k]):
    print(f'{k:22s} req {cr[k]/nr:.3f}  matched {cc[k]/nc:.3f}')
Ps.sort(); print('P (ms) q25/50/75/90', [Ps[int(q*(len(Ps)-1))] for q in (.25,.5,.75,.9)], 'n', len(Ps))
