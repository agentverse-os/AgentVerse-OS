InfluxDB is an open-source, high-performance time series database packaged with Chronograf, the official web-based visualization tool. Built specifically for handling high-frequency data, InfluxDB excels at managing and analyzing large volumes of timestamped information.

⚠️ Default admin credentials are provided for advanced CLI access. Chronograf is pre-configured with these credentials and requires no additional setup. To find these credentials you can right-click on the InfluxDB app icon from your umbrelOS homescreen and select "Show default credentials".

Why use InfluxDB?

**🚀 High Performance**
InfluxDB is a high-performance data store written specifically for time series data. It allows for high throughput ingest, compression and real-time querying.

**💡 SQL-Like Queries with InfluxQL**
InfluxDB works with InfluxQL, a SQL-like query language for interacting with data. It has been lovingly crafted to feel familiar to those coming from other SQL or SQL-like environments while also providing features specific to storing and analyzing time series data. InfluxQL supports regular expressions, arithmetic expressions, and time series-specific functions to speed up data processing.

**📦 Efficient Data Retention and Downsampling**
InfluxDB can handle millions of data points per second. Working with that much data over a long period can lead to storage concerns. InfluxDB automatically compacts data to minimize your storage space. In addition, you can easily downsample the data; keeping high-precision raw data for a limited time and storing the lower-precision, summarized data for much longer or until the end of time.

**📊 Integrated Visualization with Chronograf**
Chronograf is the administrative user interface and visualization engine of the stack. It makes it easy to setup and maintain the monitoring and alerting for your infrastructure. It's simple to use and includes templates and libraries that allow you to rapidly build dashboards with real-time visualizations of your data and to easily create alerting and automation rules.