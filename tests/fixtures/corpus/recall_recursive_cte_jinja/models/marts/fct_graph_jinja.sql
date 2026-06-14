with recursive graph as (select 1 as n union all select n + 1 from graph where n < 5) select * from graph
