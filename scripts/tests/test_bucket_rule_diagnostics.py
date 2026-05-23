import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from bucket_rule_diagnostics import (  # noqa: E402
    CLASSIFIERS,
    classify_sqlcost012,
    classify_sqlcost016,
    classify_sqlcost017,
    classify_sqlcost019,
    classify_sqlcost005,
)


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


class ClassifierRegistryTests(unittest.TestCase):
    def test_registry_covers_triage_rules(self) -> None:
        for rule_id in ("SQLCOST012", "SQLCOST016", "SQLCOST017", "SQLCOST019", "SQLCOST005"):
            self.assertIn(rule_id, CLASSIFIERS)

    def test_sqlcost017_symmetric(self) -> None:
        sql = "select * from a join b on lower(a.email) = lower(b.email)"
        self.assertEqual(classify_sqlcost017(sql), "symmetric_normalize")

    def test_sqlcost016_date_trunc(self) -> None:
        sql = "select * from t where date_trunc('day', block_time) >= current_date"
        self.assertEqual(classify_sqlcost016(sql), "date_trunc_filter")

    def test_sqlcost019_block_time(self) -> None:
        sql = """
        {% if is_incremental() %}
        select * from {{ source('dex','trades') }} where block_time >= current_date
        {% endif %}
        """
        self.assertEqual(classify_sqlcost019(sql), "block_time_in_incremental")

    def test_sqlcost005_missing(self) -> None:
        sql = "{% if is_incremental() %} select * from t {% endif %}"
        self.assertEqual(classify_sqlcost005(sql), "missing_predicate")


if __name__ == "__main__":
    unittest.main()
