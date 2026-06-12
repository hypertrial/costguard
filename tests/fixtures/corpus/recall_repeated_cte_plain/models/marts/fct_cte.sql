with base as (select id from events)
select * from base join base b on base.id = b.id join base c on base.id = c.id
