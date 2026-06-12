select
    order_id,
    customer_id,
    lower(trim(email)) as email_normalized
from {{ source('raw', 'orders') }}
