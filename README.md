# Mucab

Mucab (µcab) takes [Mecab](https://en.wikipedia.org/wiki/MeCab) data (at least `mecab-ipadic-2.7.0-20070801`), and compacts it. It operates on the compacted data:

```
Input:  うわー、それは素晴らしいです
Output: うわー、それはスバラシイです
```

```
Input:  ウィキペディア（Ｗｉｋｉｐｅｄｉａ）は誰でも編集できるフリー百科事典です
Output: ウィキペディア（Ｗｉｋｉｐｅｄｉａ）はダレでもヘンシューできるフリーヒャクカジテンです
```


The only focus of this project is to provide compact data format & compact code to transliterate Japanese Kanji.

With the cut down data, the uncompressed size changes from 52MB (30MB data + 22MB matrix.def) to 13MB.

Compressed 3.6MB for Mucab and 4.9MB for the original data (Mucab format has indices to make lookups faster)
