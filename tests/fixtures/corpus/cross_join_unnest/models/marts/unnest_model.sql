select t.x
from {{ ref('stg_orders') }} as base
cross join unnest(base.items) as t(x)
