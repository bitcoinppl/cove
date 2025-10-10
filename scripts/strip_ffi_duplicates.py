#!/usr/bin/env python3
"""
Strip duplicate FFI infrastructure from UniFFI-generated Kotlin files.
Keeps only the actual type definitions (enums, sealed classes, data classes, etc.)
and removes FFI boilerplate (RustBuffer, FfiConverter*, UniffiLib, etc.)
"""

import sys
import re

def should_skip_block(line):
    """Check if this line starts a block we want to skip"""
    skip_patterns = [
        r'^@Structure\.FieldOrder',
        r'^open class RustBuffer',
        r'^class RustBufferByReference',
        r'^internal class RustBufferByReference',
        r'^object FfiConverter',
        r'^internal object FfiConverter',
        r'^class UniffiRustCallStatus',
        r'^internal class UniffiRustCallStatus',
        r'^internal object UniffiLib',
        r'^object UniffiLib',
        r'^object UniffiWithHandle',
        r'^internal object UniffiWithHandle',
        r'^private object UniffiHandleMap',
        r'^object FfiConverterString',
        r'^internal object FfiConverterString',
    ]

    for pattern in skip_patterns:
        if re.match(pattern, line):
            return True
    return False

def process_file(input_path):
    """Process a Kotlin file and remove FFI infrastructure"""
    with open(input_path, 'r') as f:
        lines = f.readlines()

    output_lines = []
    skip_until_brace_closes = False
    brace_depth = 0

    for line in lines:
        # always keep package and imports
        if line.startswith('package ') or line.startswith('import '):
            output_lines.append(line)
            continue

        # check if we should start skipping
        if not skip_until_brace_closes and should_skip_block(line.strip()):
            skip_until_brace_closes = True
            brace_depth = 0

        if skip_until_brace_closes:
            # count braces to know when the block ends
            brace_depth += line.count('{') - line.count('}')

            # if we've closed all braces, stop skipping after this line
            if brace_depth <= 0:
                skip_until_brace_closes = False
            continue

        # keep this line
        output_lines.append(line)

    # write back
    with open(input_path, 'w') as f:
        f.writelines(output_lines)

if __name__ == '__main__':
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <kotlin-file>")
        sys.exit(1)

    for file_path in sys.argv[1:]:
        print(f"Processing {file_path}")
        process_file(file_path)
