select id from orders where id in (select order_id from returns)
