select u.id, p.max_amount from users u join payment_totals p on u.id = p.user_id
