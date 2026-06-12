select id from orders where status in (select status from allowed_statuses)
