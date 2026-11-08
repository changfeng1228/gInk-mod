import os, subprocess, shutil

root = 'D:/DSHXM/gInk-mod'
old_exe = root + '/bin/gInk.exe'
new_exe = root + '/build/gInk.exe'
a_dir = root + '/build/_old_res'
b_dir = root + '/build/_new_res'
for d in (a_dir, b_dir):
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)

ext = root + '/build/extract.exe'
subprocess.run([ext, old_exe, a_dir], check=True, capture_output=True)
subprocess.run([ext, new_exe, b_dir], check=True, capture_output=True)


def names(d):
    out = {}
    for f in os.listdir(d):
        # extract.exe writes using the manifest name as the file name, but LoadFrom-relative
        # paths show up as full paths in the console output only; on disk names are plain
        out[f] = os.path.getsize(os.path.join(d, f))
    return out


a, b = names(a_dir), names(b_dir)
print('old resources:', sorted(a))
print('new resources:', sorted(b))
for k in sorted(set(a) | set(b)):
    same = 'same' if a.get(k) == b.get(k) else 'DIFF'
    print('%-45s %8s %8s  %s' % (k, a.get(k), b.get(k), same))
