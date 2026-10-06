"""Last pass / first fail thickness per weapon from passmap.py output."""
import sys, json
rows = [json.loads(l) for l in open(sys.argv[1]) if l.startswith('{')]
by = {}
for r in rows:
    if not r['layers'] or len(r['layers']) < 2 or r['layers'][1][4].startswith('TOOLS'):
        continue
    l1 = r['layers'][0]
    beyond = any(d > l1[0] + l1[1] + 1 for d in r['imps'][1:])
    by.setdefault(r['w'], []).append((round(l1[1], 2), l1[2], l1[3], beyond, len(r['layers']) > 1 and round(r['layers'][1][0] - l1[0] - l1[1], 1)))
for w, v in by.items():
    v.sort()
    print(w, 'max pass', max([x[0] for x in v if x[3]], default=None), 'min fail', min([x[0] for x in v if not x[3]], default=None))
    print('  ', ' '.join(f"{x[0]}{'+' if x[3] else '-'}" for x in v))
