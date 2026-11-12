"""Function boundaries in native ARM64 Mach-O release assembly."""

import subprocess
import unittest
from pathlib import Path


class NativeAssembly(unittest.TestCase):
    def inspect(self, assembly):
        return subprocess.check_output(
            ["awk", "-f", str(Path(__file__).with_name("simd.awk"))],
            input=assembly,
            text=True,
        )

    def test_macho_function_ends_at_cfi_endproc(self):
        assembly = (Path(__file__).parent / "fixtures/swap-macho.s").read_text()
        self.assertNotIn("Lfunc_end", assembly)
        result = self.inspect(assembly)
        self.assertIn("rev64.16b", result)
        self.assertTrue(result.endswith("\t.cfi_endproc\n"))
        self.assertNotIn("from_vdata_type:", result)

    def test_next_function_label_ends_a_block_without_an_end_directive(self):
        assembly = (Path(__file__).parent / "fixtures/swap-macho.s").read_text()
        result = self.inspect(assembly.replace("\t.cfi_endproc\n", ""))
        self.assertIn("LBB147_17:", result)
        self.assertNotIn("from_vdata_type:", result)
        self.assertNotIn("[sp, #-32]!", result)


if __name__ == "__main__":
    unittest.main()
