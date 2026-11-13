import json
import math
from pathlib import Path
import statistics
import sys

root=Path(sys.argv[1])
runs=json.loads((root/'startup-runs.json').read_text(encoding='utf-8-sig'))
assert len(runs)==40, f'Expected 40 real launches, got {len(runs)}'

def stats(values):
    assert values and all(v >= 0 for v in values)
    values=sorted(values)
    return {'n':len(values),'medianMs':round(statistics.median(values),3),
            'p95Ms':round(values[math.ceil(len(values)*.95)-1],3),
            'minMs':round(values[0],3),'maxMs':round(values[-1],3)}

result={'method':'Fresh processes and fresh profiles, OS caches retained; nearest-rank P95',
        'variants':{}}
for variant in ['before','after']:
    rows=[r for r in runs if r['variant']==variant]
    assert len(rows)==20
    assert all(r['timing']['dpr']==1 for r in rows)
    totals=[r['processToFirstFrameMs'] for r in rows]
    warmup=[r['timing']['stagesUs']['warmUpFonts']/1000 for r in rows]
    asset=[r['timing']['stagesUs'].get('fontAsset:NotoSansSC',0)/1000 for r in rows]
    result['variants'][variant]={'firstObservedLaunchMs':totals[0],
        'processToFirstFrame':stats(totals),'subsequentLaunches':stats(totals[1:]),
        'warmUpFonts':stats(warmup),'notoReadAndRegister':stats(asset)}
deltas=[]
for pair in range(20):
    rows={r['variant']:r for r in runs if r['pair']==pair}
    assert len(rows)==2
    deltas.append(rows['after']['processToFirstFrameMs']-rows['before']['processToFirstFrameMs'])
result['pairedDeltaMedianMs']=round(statistics.median(deltas),3)
result['pairedDeltaRangeMs']=[round(min(deltas),3),round(max(deltas),3)]
(root/'startup-summary.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
print(json.dumps(result,indent=2))
