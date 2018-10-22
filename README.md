# A Web Demo for Scientific Paragraph Classification

### Preview available at:
https://corpora.mathweb.org/classify_paragraph

### Method
 * [latexml](https://github.com/brucemiller/LaTeXML) converts the source into an HTML5 document
 * [llamapun](https://github.com/KWARC/llamapun) tokenizes the first paragraph into a plain-text representation with sub-formula lexemes
 * [tensorflow](https://github.com/tensorflow/rust) executes a pre-trained BiLSTM model to obtain the likelihoods of 6 potential amsthm classes

### Details

The current deployed model is a Keras **BiLSTM(128)→BiLSTM(64)→LSTM(64)**, with a **Dense(8)** softmax output.

The input layer is embedded via the [arxmliv 08.2018 GloVe embeddings](https://sigmathling.kwarc.info/resources/arxmliv-embeddings-082018/),
as well as padded/truncated to a maximum length of 480 words. 
A paragraph is hence a fixed `(480,300)` matrix, as passed into the bilstm layer.

The specific model in this demo was trained on **4.08 million** paragraphs from the [arxmliv 08.2018 dataset](https://sigmathling.kwarc.info/resources/arxmliv-dataset-082018/),
and tested on **1.02 million paragraphs** respectively, obtaining a **0.95 F1 score**.
The base rate baseline was 0.62, the frequency of the "proposition" class.

The classification labels originated from the author-provided LaTeX markup via the \newtheorem macro and environments, in the paper sources as submitted to arXiv. Only the first paragraph in each environment was used. The "introduction" and "related work" classes were obtained from sections of the same name.

For practical evaluation, a likelihood threshold of 0.70—0.75 could be used, where entries with smaller likelihoods can be considered as an "other" label.

### Notes

These results are currently unpublished, and this repository is meant as a preview workspace for using a [rocket](https://rocket.rs/) web service with tensorflow-rust for this type of demonstration.

Also, the server is running a CPU version of the underling `CuDNNLSTM` layers, so expect rather slow runtimes, of 15-20 seconds for the tensorflow stage.
