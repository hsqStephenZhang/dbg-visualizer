"""Host-side validation tests; no debugger or target process required."""
import importlib.util
from pathlib import Path
import struct
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("dbgvis_core", ROOT / "debugger/common/core.py")
core = importlib.util.module_from_spec(spec)
spec.loader.exec_module(core)


class FakeBackend:
    def __init__(self):
        self.memory = bytearray(20000)
        self.identity_value = 1
        self.process = 1
        self.called = 0
        self.value_name = "demo::Point"
        self.value_size = 8
        self.response_sequence_offset = 0
        self.response_written = 2
        self.memory[100:212] = struct.pack("<8s8I9Q", b"DBGVIS01", 1, 112, 8, 1, 112, 88, 48, 304,
                                           1000, 1, 2000, 2100, 2200, 4096, 7000, 128, 9000)
        self.memory[1000:1112] = struct.pack("<14Q", 1500, 11, 0, 0, 0, 0, 8, 4, 3, 0, 0, 0, 0, 0)
        self.memory[1500:1511] = b"demo::Point"
        self.memory[2200:2202] = b"ok"

    def identity(self): return self.identity_value
    def process_key(self): return self.process
    def module_address(self): return 100
    def read(self, address, length): return bytes(self.memory[address:address + length])
    def write(self, address, data): self.memory[address:address + len(data)] = data
    def value_type(self, value): return self.value_name, self.value_size, 4
    def owns_type(self, value): return True
    def value_address(self, value): return 12000
    def ensure_callable(self, options): pass
    def call(self, function, request, options):
        self.called += 1
        sequence = struct.unpack_from("<Q", self.memory, request + 16)[0]
        self.memory[2100:2148] = struct.pack("<6Q", sequence + self.response_sequence_offset, 0,
                                          self.response_written, 0, 0, 0)
        return 0


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.backend = FakeBackend()
        self.session = core.Session(self.backend)

    def test_unknown_abi_does_not_execute_or_write(self):
        self.backend.memory[108:112] = struct.pack("<I", 99)
        with self.assertRaisesRegex(core.VisualizerError, "ABI_MISMATCH"):
            self.session.summary(None, {"summary.mode": "debug"})
        self.assertEqual(self.backend.called, 0)

    def test_same_layout_is_not_enough_to_match_a_type(self):
        self.backend.value_name = "demo::OtherPoint"
        with self.assertRaises(core.NoMatch): self.session.match(None)
        self.backend.value_name = "demo::Point"
        self.backend.value_size = 16
        with self.assertRaises(core.NoMatch): self.session.match(None)
        self.backend.value_size = 8
        entry = self.session.match(None)
        self.session.entries.append(entry)
        with self.assertRaises(core.NoMatch): self.session.match(None)
        self.assertEqual(self.backend.called, 0)

    def test_limits_are_checked_before_target_call(self):
        for capacity in (0, -1, 4097):
            with self.assertRaisesRegex(core.VisualizerError, "buffer size"):
                self.session.summary(None, {"summary.mode": "debug", "summary.buffer_bytes": capacity})
        self.assertEqual(self.backend.called, 0)

    def test_response_mismatch_disables_calls_until_new_process(self):
        self.backend.response_sequence_offset = 1
        with self.assertRaisesRegex(core.VisualizerError, "inconsistent"):
            self.session.summary(None, {"summary.mode": "debug"})
        self.assertTrue(self.session.poisoned)
        self.backend.identity_value += 1  # Module change alone must not unpoison the process.
        with self.assertRaisesRegex(core.VisualizerError, "interrupted"):
            self.session.summary(None, {"summary.mode": "debug"})
        self.backend.process += 1
        self.backend.response_sequence_offset = 0
        self.assertEqual(self.session.summary(None, {"summary.mode": "debug"}), "ok")

    def test_oversized_response_is_not_read(self):
        self.backend.response_written = 4097
        with self.assertRaisesRegex(core.VisualizerError, "bounds"):
            self.session.summary(None, {"summary.mode": "debug"})

    def test_configuration_precedence_and_validation(self):
        options = core.Config()
        options.set("summary.mode", "debug")
        options.set("summary.mode", "display", "demo::Point")
        self.assertEqual(options.resolve("demo::Point")["summary.mode"], "display")
        self.assertEqual(options.resolve("demo::Point", {"summary.mode": "native"})["summary.mode"], "native")
        self.assertEqual(options.resolve("other")["summary.mode"], "debug")
        for key, value in [("children.page_size", "65"), ("summary.alternate", "maybe"),
                           ("summary.mode", "guess"), ("summary.buffer_bytes", "-1")]:
            with self.assertRaises(core.VisualizerError): options.set(key, value)


if __name__ == "__main__": unittest.main()
