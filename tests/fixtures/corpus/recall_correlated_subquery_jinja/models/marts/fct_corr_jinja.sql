select id from orders o
where exists (select 1 from line_items li where li.order_id = o.id)
