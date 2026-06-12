select u.id, (select max(amount) from payments p where p.user_id = u.id) as max_pay
from users u
