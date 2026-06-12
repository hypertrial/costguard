select u.id, (select count(*) from {{ ref('stg_orders') }} o where o.user_id = u.id) as n
from users u
