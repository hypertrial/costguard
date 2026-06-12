with c as (select id from {{ ref('stg_events') }})
select c.id from c join c d on c.id = d.id join c e on c.id = e.id
