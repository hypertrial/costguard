select p.price
from {{ ref('stg_prices') }} p
join {{ ref('stg_logs') }} le
  on p.minute = date_trunc('minute', le.block_time)
