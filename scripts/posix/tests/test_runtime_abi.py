"""Exercise real target symbol-version bindings with the host ELF loader."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class RuntimeAbiTests(unittest.TestCase):
    def test_identity_scope_and_limits_retain_both_target_versions(self):
        runtime = Path(__file__).parents[1] / "runtime"
        with tempfile.TemporaryDirectory() as directory:
            library = Path(directory) / "compat.so"
            # Enable target aliases in a native ELF. The exercised paths
            # contain no ISA instructions or host credential changes.
            subprocess.run(
                ["cc", "-std=gnu99", "-fPIC", "-shared", "-D__riscv",
                 str(runtime / "smros_posix_compat.c"),
                 f"-Wl,--version-script,{runtime / 'smros_posix_compat.map'}",
                 "-ldl", "-o", str(library)], check=True, timeout=30,
            )
            probe = r'''
import ctypes as c
import os
import sys
lib = c.CDLL(sys.argv[1], mode=c.RTLD_LOCAL)
loader = c.CDLL(None)
loader.dlvsym.argtypes = (c.c_void_p, c.c_char_p, c.c_char_p)
loader.dlvsym.restype = c.c_void_p
class Info(c.Structure):
    _fields_ = [('file', c.c_char_p), ('base', c.c_void_p),
                ('name', c.c_char_p), ('address', c.c_void_p)]
loader.dladdr.argtypes = (c.c_void_p, c.POINTER(Info))
version = sys.argv[2].encode()
def resolve(name):
    ptr = loader.dlvsym(lib._handle, name.encode(), version)
    assert ptr, f'missing {name}@{version.decode()}'
    owner = Info()
    assert loader.dladdr(ptr, c.byref(owner))
    assert owner.file.decode() == sys.argv[1], f'{name} bypassed compat'
    return ptr
for name in ('getuid', 'geteuid', 'setuid', 'seteuid', 'setpwent', 'endpwent',
             'getpwent', 'sysconf', 'pthread_attr_setscope', 'pthread_attr_getscope'):
    resolve(name)
reset = c.CFUNCTYPE(None)(resolve('setpwent'))
next_user = c.CFUNCTYPE(c.c_void_p)(resolve('getpwent'))
reset()
root, user = next_user(), next_user()
assert root and user and root != user and next_user() is None
reset()
assert next_user() == root
conf = c.CFUNCTYPE(c.c_long, c.c_int)(resolve('sysconf'))
assert conf(os.sysconf_names['SC_SEM_NSEMS_MAX']) > 0
# Aligned storage larger than glibc's pthread_attr_t, initialized by libc.
attr = (c.c_long * 16)()
init = c.CFUNCTYPE(c.c_int, c.c_void_p)(resolve('pthread_attr_init'))
destroy = c.CFUNCTYPE(c.c_int, c.c_void_p)(resolve('pthread_attr_destroy'))
set_scope = c.CFUNCTYPE(c.c_int, c.c_void_p, c.c_int)(resolve('pthread_attr_setscope'))
get_scope = c.CFUNCTYPE(c.c_int, c.c_void_p, c.POINTER(c.c_int))(resolve('pthread_attr_getscope'))
assert init(attr) == 0
for scope in (1, 0):  # glibc PTHREAD_SCOPE_PROCESS, PTHREAD_SCOPE_SYSTEM
    actual = c.c_int(-1)
    assert set_scope(attr, scope) == 0
    assert get_scope(attr, c.byref(actual)) == 0 and actual.value == scope
assert destroy(attr) == 0
'''
            env = {k: v for k, v in os.environ.items()
                   if k not in ("LD_PRELOAD", "SMROS_POSIX_TEST_USER")}
            for version in ("GLIBC_2.17", "GLIBC_2.27"):
                with self.subTest(version=version):
                    result = subprocess.run(
                        [sys.executable, "-c", probe, str(library), version],
                        env=env, capture_output=True, text=True, timeout=5,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
