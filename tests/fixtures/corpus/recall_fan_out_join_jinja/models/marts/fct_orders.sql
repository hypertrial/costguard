select * from orders join {{ ref('dim_users') }} u on orders.email = u.email
