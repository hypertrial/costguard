select l.*
from licenses l
left join (
    select a.user_id, l.date
    from licenses l
    join activity a
        on l.server_id = a.user_id
        and a.timestamp <= l.date
    group by 1, 2
) a
    on l.server_id = a.user_id
    and a.date = l.date
