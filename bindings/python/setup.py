# The package is pure-Python ctypes but carries a platform-specific prebuilt dylib, so the
# wheel must be impure + platform-tagged, yet abi-agnostic (py3-none-<plat>). BinaryDistribution
# (has_ext_modules -> True) keeps the files at the wheel root; get_tag forces py3/none and takes
# the platform from OBOL_WHEEL_PLAT so each CI leg stamps its exact arch.
import os

from setuptools import setup
from setuptools.dist import Distribution

try:  # modern path first (wheel >=0.46 deprecates its own bdist_wheel shim)
    from setuptools.command.bdist_wheel import bdist_wheel as _bdist_wheel
except ImportError:
    from wheel.bdist_wheel import bdist_wheel as _bdist_wheel


class BinaryDistribution(Distribution):
    def has_ext_modules(self):
        return True


class bdist_wheel(_bdist_wheel):
    def get_tag(self):
        _py, _abi, plat = super().get_tag()
        return ("py3", "none", os.environ.get("OBOL_WHEEL_PLAT", plat))


setup(distclass=BinaryDistribution, cmdclass={"bdist_wheel": bdist_wheel})
