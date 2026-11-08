import glob, os
files = sorted(glob.glob('src/*.cs')) + sorted(glob.glob('src/Properties/*.cs'))
lines = ['"%s"' % ('D:/DSHXM/gInk-mod/' + f) for f in files]
for r in sorted(os.listdir('build/res')):
    lines.append('-resource:"%s"' % r)
open('build/sources.rsp', 'w').write('\n'.join(lines) + '\n')
print('\n'.join(lines[-8:]))
