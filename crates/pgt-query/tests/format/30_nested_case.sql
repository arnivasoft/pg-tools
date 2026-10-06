select case when a > 0 then case when b > 0 then 'both positive values found here' else 'only a is positive' end else 'a is not positive at all here' end as label from t
