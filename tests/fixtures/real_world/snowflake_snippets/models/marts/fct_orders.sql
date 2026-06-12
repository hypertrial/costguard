select
    order_id,
    count(distinct customer_id) as customers
from {{ ref('stg_orders') }}
group by 1
