import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from bucket_rule_diagnostics import classify_sqlcost012  # noqa: E402


class ClassifySqlcost012Tests(unittest.TestCase):
    def test_cross_join_unnest(self) -> None:
        sql = "SELECT t.x FROM arr CROSS JOIN UNNEST(arr) AS t(x)"
        self.assertEqual(classify_sqlcost012(sql), "cross_join_unnest")

    def test_string_literal_fp(self) -> None:
        sql = "SELECT 'cross join in string' AS note FROM t"
        self.assertEqual(classify_sqlcost012(sql), "string_literal_fp")

    def test_real_cross_join(self) -> None:
        sql = "SELECT * FROM a CROSS JOIN b"
        self.assertEqual(classify_sqlcost012(sql), "cross_join_explicit")

    def test_comma_join(self) -> None:
        sql = "SELECT * FROM a, b"
        self.assertEqual(classify_sqlcost012(sql), "comma_join")

    def test_subquery_comma_fp(self) -> None:
        sql = "SELECT * FROM (SELECT a, b FROM x), y"
        self.assertEqual(classify_sqlcost012(sql), "subquery_comma_fp")


if __name__ == "__main__":
    unittest.main()
