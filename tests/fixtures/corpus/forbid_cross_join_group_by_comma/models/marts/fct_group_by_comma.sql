select s.id
from staging_a s
inner join staging_b l on l.id = s.id
group by s.id, s.block_number
