export default {
	number_locale: new Intl.NumberFormat().resolvedOptions().locale,
	date_locale: new Intl.DateTimeFormat().resolvedOptions().locale,
	number: (1234.5).toLocaleString()
};
