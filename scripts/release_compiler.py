#!/usr/bin/env python3
"""Compiler shim: remap native paths without embedding them in OpenSSL CFLAGS."""
import json
import os
from pathlib import Path
import shlex
import sys


def main():
    variable = 'AEGIS_RELEASE_CXX' if Path(sys.argv[0]).name.endswith('cxx') else 'AEGIS_RELEASE_CC'
    command = shlex.split(os.environ[variable])
    mappings = json.loads(os.environ['AEGIS_RELEASE_PATH_MAPS'])
    flags = [f'-ffile-prefix-map={source}={destination}' for source, destination in mappings]
    arguments = []
    for argument in sys.argv[1:]:
        # Vendored OpenSSL also exposes its build-only module/engine install
        # directories via version strings. These are not runtime search paths
        # supplied by the operator; keep their placeholder prefix neutral.
        if argument.startswith(('-DENGINESDIR=', '-DMODULESDIR=', '-DOPENSSLDIR=',
                                'ENGINESDIR=', 'MODULESDIR=', 'OPENSSLDIR=')):
            for source, destination in mappings:
                argument = argument.replace(source, destination)
        arguments.append(argument)
    os.execvp(command[0], [*command, *flags, *arguments])


if __name__ == '__main__':
    main()
