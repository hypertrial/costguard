# SQLCOST013: Unpartitioned window function

Detects `OVER ()` and window functions without `PARTITION BY`.

Partition windows by the natural entity key when possible.
