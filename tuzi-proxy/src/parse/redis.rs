// pub fn redis_args_count(input: &[u8]) -> IResult<&[u8], usize> {
//     delimited(
//         tag("*"),
//         map(digit1, |s| str::from_utf8(s).unwrap().parse().unwrap()),
//         crlf,
//     )(input)
// }
