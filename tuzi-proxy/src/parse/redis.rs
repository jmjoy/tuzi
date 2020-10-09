// pub fn redis_args_count(input: &[u8]) -> IResult<&[u8], usize> {
//     delimited(
//         tag("*"),
//         map(digit1, |s| str::from_utf8(s).unwrap().parse().unwrap()),
//         crlf,
//     )(input)
// }

use crate::{error::TuziResult, parse::ParserDelivery};

pub async fn parse(mut request_parser_delivery: ParserDelivery) -> TuziResult<()> {
    todo!()
}
