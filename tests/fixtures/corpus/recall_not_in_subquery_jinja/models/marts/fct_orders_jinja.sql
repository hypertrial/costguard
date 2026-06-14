select id from orders where id not in (select order_id from {{ ref('stg_returns') }})
