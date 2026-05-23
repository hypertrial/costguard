{% macro explicit_join() %}
select s.id
from {{ ref('stg_a') }} s
inner join {{ ref('stg_b') }} l on l.id = s.id
group by s.id, s.block_number
{% endmacro %}
